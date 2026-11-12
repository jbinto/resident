use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::{
    HIT_PART_DIVIDER, HIT_PART_MAX_SIZE, MAX_FREQUENCY_FACTOR, MAX_TIME_FACTOR,
    MIN_FREQUENCY_FACTOR, MIN_SECONDS_WITH_MATCH, MIN_TIME_FACTOR, QUERY_PROFILE, bin_to_hz,
    bins_to_seconds,
};
use crate::store::{LookupTable, StoredHit};
use crate::{Error, Fingerprint, Result, Store};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceHit {
    pub q_t: u32,
    pub ref_t: u32,
    pub q_seconds: f64,
    pub ref_seconds: f64,
    pub original_hash: u64,
    pub matched_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistogramBin {
    pub bin: i64,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DensityBin {
    pub second: u32,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub hits: Vec<EvidenceHit>,
    pub offset_top: Vec<HistogramBin>,
    pub per_second: Vec<DensityBin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchRow {
    pub ref_key: String,
    pub q_start: f64,
    pub q_stop: f64,
    pub ref_start: f64,
    pub ref_stop: f64,
    pub score: usize,
    pub time_factor: f64,
    pub pitch_factor: f64,
    pub sec_with_match: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

#[derive(Clone, Copy, Debug)]
struct Hit {
    ref_t: u32,
    ref_f: u16,
    q_t: u32,
    q_f: u16,
    original_hash: u64,
    matched_hash: u64,
}

impl Hit {
    fn delta(self) -> i64 {
        i64::from(self.ref_t) - i64::from(self.q_t)
    }
}

pub struct Matcher<'a> {
    store: &'a Store,
    /// Resource ids whose hits never reach a voter (sorted). Their postings still count toward
    /// the global matching-hash population, so every other resource's answer is unchanged.
    skip: Vec<u32>,
}

impl<'a> Matcher<'a> {
    pub const fn new(store: &'a Store) -> Self {
        Self {
            store,
            skip: Vec::new(),
        }
    }

    /// A matcher that never votes the named resources (SELF-SKIP). Keys absent from the store
    /// are ignored: a probe's own key need not be in the target. Rows for every other resource
    /// are identical to `new`'s, because skipped postings still mark their hashes present.
    pub fn skipping(store: &'a Store, keys: &[&str]) -> Self {
        let mut skip: Vec<u32> = keys
            .iter()
            .filter_map(|key| store.resource(key).ok().map(|resource| resource.id))
            .collect();
        skip.sort_unstable();
        skip.dedup();
        Self { store, skip }
    }

    pub fn match_prints(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        self.match_restricted(prints, k, include_evidence, false, None)
    }

    pub fn match_prints_multiline(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        self.match_restricted(prints, k, include_evidence, true, None)
    }

    /// `match_prints` with this store's lookups already batched in `table` (built by
    /// `Store::lookup_table` over a superset of `prints`' hashes); identical results.
    pub fn match_prints_with(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
        table: &LookupTable,
    ) -> Result<Vec<MatchRow>> {
        self.match_restricted_with(prints, k, include_evidence, false, None, Some(table))
    }

    /// `match_prints_multiline` with this store's lookups already batched in `table` (built by
    /// `Store::lookup_table` over a superset of `prints`' hashes); identical results.
    pub fn match_prints_multiline_with(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
        table: &LookupTable,
    ) -> Result<Vec<MatchRow>> {
        self.match_restricted_with(prints, k, include_evidence, true, None, Some(table))
    }

    pub fn match_resource(
        &self,
        prints: &[Fingerprint],
        ref_key: &str,
        include_evidence: bool,
    ) -> Result<Option<MatchRow>> {
        let id = self.store.resource(ref_key)?.id;
        Ok(self
            .match_restricted(prints, 1, include_evidence, false, Some(id))?
            .into_iter()
            .next())
    }

    pub fn match_resource_multiline(
        &self,
        prints: &[Fingerprint],
        ref_key: &str,
        k: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        let id = self.store.resource(ref_key)?.id;
        self.match_restricted(prints, k, include_evidence, true, Some(id))
    }

    fn match_restricted(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
        multi_line: bool,
        only_resource: Option<u32>,
    ) -> Result<Vec<MatchRow>> {
        self.match_restricted_with(prints, k, include_evidence, multi_line, only_resource, None)
    }

    fn match_restricted_with(
        &self,
        prints: &[Fingerprint],
        k: usize,
        include_evidence: bool,
        multi_line: bool,
        only_resource: Option<u32>,
        table: Option<&LookupTable>,
    ) -> Result<Vec<MatchRow>> {
        if prints.is_empty() {
            return Err(Error::BadRequest("probe prints must not be empty".into()));
        }
        if k == 0 {
            return Err(Error::BadRequest("k must be greater than zero".into()));
        }

        // Panako queues every input hash but its hash-keyed probe map retains the last (t,f)
        // for duplicates. Preserve that compatibility quirk inside this module only.
        let mut probe = HashMap::<u64, ProbeHash>::new();
        for (index, &print) in prints.iter().enumerate() {
            let entry = probe.entry(print.hash).or_insert(ProbeHash {
                last: print,
                copies: 0,
                first: index,
            });
            entry.last = print;
            entry.copies += 1;
        }
        let mut hashes: Vec<_> = probe.keys().copied().collect();
        hashes.sort_unstable();

        // Each probe hash's stored hits, borrowed from a batched table where one exists. A full-
        // store question without a caller's table builds one: the same hits as per-hash `lookup`
        // (Store::lookup_table's contract), walking each shard once instead of bisecting it per
        // hash.
        let owned_table;
        let owned_lists: Vec<(u64, Vec<StoredHit>, bool)>;
        let mut lists: Vec<(u64, &[StoredHit])> = Vec::with_capacity(hashes.len());
        if let Some(resource_id) = only_resource {
            owned_lists = hashes
                .par_iter()
                .map(|&hash| {
                    let hits = self.store.lookup_resource(hash, resource_id)?;
                    // Only globally present hashes enter Panako's map (its iteration capacity).
                    Ok((hash, hits, self.store.has_match(hash)?))
                })
                .collect::<Result<_>>()?;
            lists.extend(
                owned_lists
                    .iter()
                    .filter(|(_, _, present)| *present)
                    .map(|(hash, hits, _)| (*hash, &hits[..])),
            );
        } else {
            let table = match table {
                Some(table) => table,
                None => {
                    owned_table = self.store.lookup_table(&hashes)?;
                    &owned_table
                }
            };
            // A caller's table should cover the probe; any hash it lacks is looked up directly.
            owned_lists = hashes
                .iter()
                .filter(|&&hash| table.entry(hash).is_none())
                .map(|&hash| {
                    let hits = self.store.lookup(hash)?;
                    let present = !hits.is_empty();
                    Ok((hash, hits, present))
                })
                .collect::<Result<_>>()?;
            let mut missing = owned_lists.iter();
            for &hash in &hashes {
                // A table may hold only some resources' hits (`lookup_table_among`), so presence
                // comes from the table, not from whether its hits are empty.
                let (hits, present) = match table.entry(hash) {
                    Some(entry) => entry,
                    None => {
                        let (_, hits, present) = missing.next().expect("looked up above");
                        (&hits[..], *present)
                    }
                };
                if present {
                    lists.push((hash, hits));
                }
            }
        }
        let capacity = java_hash_map_capacity(lists.len());
        lists.sort_by_key(|(hash, _)| (java_long_bucket(*hash, capacity), probe[hash].first));

        let mut groups = self.gather(&lists, &probe, only_resource)?;
        let votes: Vec<Result<Vec<MatchRow>>> = groups
            .split()
            .into_par_iter()
            .with_min_len(VOTE_CHUNK)
            .map(|(resource_id, hits)| {
                // Stable: equal-time hits keep their gathered order, which Panako's vote sees.
                hits.sort_by_key(|hit| hit.q_t);
                if multi_line {
                    self.vote_lines(resource_id, hits, k, include_evidence)
                } else {
                    Ok(self
                        .vote(resource_id, hits, include_evidence)?
                        .into_iter()
                        .collect())
                }
            })
            .collect();
        let mut rows = Vec::new();
        for vote in votes {
            rows.extend(vote?);
        }
        rows.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.ref_key.cmp(&b.ref_key))
                .then_with(|| a.q_start.total_cmp(&b.q_start))
                .then_with(|| a.ref_start.total_cmp(&b.ref_start))
        });
        rows.truncate(k);
        Ok(rows)
    }

    /// Every voting resource's hits, contiguous and in the order the reference path appends
    /// them: probe hashes in `lists` order, each stored hit repeated once per probe copy. A
    /// counting pass first sizes each resource and drops those under the unfiltered gate before
    /// any hit is written, so the ~4/5 of touched resources that can never vote cost one counter
    /// increment per posting instead of a vector.
    fn gather(
        &self,
        lists: &[(u64, &[StoredHit])],
        probe: &HashMap<u64, ProbeHash>,
        only_resource: Option<u32>,
    ) -> Result<Groups> {
        const DROPPED: u32 = u32::MAX;
        let votes = |id: u32| {
            only_resource.is_none_or(|only| only == id) && self.skip.binary_search(&id).is_err()
        };
        let limit = self.store.resource_id_limit();
        let mut cursor = vec![0_u32; limit];
        for &(hash, hits) in lists {
            let copies = probe[&hash].copies as u32;
            for stored in hits {
                let slot = cursor.get_mut(stored.resource_id as usize).ok_or_else(|| {
                    Error::InvalidStore(format!(
                        "posting references unknown resource {}",
                        stored.resource_id
                    ))
                })?;
                if votes(stored.resource_id) {
                    *slot += copies;
                }
            }
        }
        let mut spans = Vec::new();
        let mut total = 0_u32;
        for (id, slot) in cursor.iter_mut().enumerate() {
            let count = *slot;
            if count as usize >= QUERY_PROFILE.min_hits_unfiltered {
                spans.push((id as u32, count as usize));
                *slot = total;
                total += count;
            } else {
                *slot = DROPPED;
            }
        }
        let blank = Hit {
            ref_t: 0,
            ref_f: 0,
            q_t: 0,
            q_f: 0,
            original_hash: 0,
            matched_hash: 0,
        };
        let mut hits = vec![blank; total as usize];
        for &(hash, stored_hits) in lists {
            let entry = &probe[&hash];
            let query = entry.last;
            for stored in stored_hits {
                let slot = &mut cursor[stored.resource_id as usize];
                if *slot == DROPPED || !votes(stored.resource_id) {
                    continue;
                }
                let at = *slot as usize;
                hits[at..at + entry.copies].fill(Hit {
                    ref_t: stored.t,
                    ref_f: stored.f,
                    q_t: query.t,
                    q_f: query.f,
                    original_hash: hash,
                    matched_hash: stored.matched_hash,
                });
                *slot += entry.copies as u32;
            }
        }
        Ok(Groups { hits, spans })
    }

    /// `sorted` is one resource's hits sorted by probe time (stably, from gathered order).
    fn vote_lines(
        &self,
        resource_id: u32,
        sorted: &[Hit],
        limit: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        let mut rows = Vec::new();
        // Most resources find no line at all: vote on the gathered slice and copy only once a
        // line has to be removed. Removing hits keeps the rest sorted, so no re-sort is needed.
        let mut remaining: Option<Vec<Hit>> = None;
        loop {
            let current = remaining.as_deref().unwrap_or(sorted);
            if current.len() < QUERY_PROFILE.min_hits_unfiltered || rows.len() >= limit {
                break;
            }
            let Some(mut row) = self.vote(resource_id, current, true)? else {
                break;
            };
            let evidence = row
                .evidence
                .take()
                .expect("multiline vote always requests internal evidence");
            let mut selected = HashMap::<(u32, u32, u64, u64), usize>::new();
            for hit in &evidence.hits {
                *selected
                    .entry((hit.q_t, hit.ref_t, hit.original_hash, hit.matched_hash))
                    .or_default() += 1;
            }
            let mut next = remaining.take().unwrap_or_else(|| sorted.to_vec());
            next.retain(|hit| {
                let key = (hit.q_t, hit.ref_t, hit.original_hash, hit.matched_hash);
                let Some(count) = selected.get_mut(&key) else {
                    return true;
                };
                if *count == 0 {
                    true
                } else {
                    *count -= 1;
                    false
                }
            });
            remaining = Some(next);
            if include_evidence {
                row.evidence = Some(evidence);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    /// `hits` must be sorted by probe time (stably, from gathered order), as Panako's are.
    fn vote(
        &self,
        resource_id: u32,
        hits: &[Hit],
        include_evidence: bool,
    ) -> Result<Option<MatchRow>> {
        let part_len = HIT_PART_MAX_SIZE.min(
            QUERY_PROFILE
                .min_hits_unfiltered
                .max(hits.len() / HIT_PART_DIVIDER),
        );
        let first = &hits[..part_len];
        let last = &hits[hits.len() - part_len..];
        let y1 = modal_delta(first);
        let y2 = modal_delta(last);
        let first_hit = first
            .iter()
            .find(|hit| hit.delta() == y1)
            .expect("mode comes from first hits");
        let last_hit = last
            .iter()
            .rev()
            .find(|hit| hit.delta() == y2)
            .expect("mode comes from last hits");
        // These are deliberately f32: Panako performs the line fit in Java floats. Hits on
        // the inclusive two-bin residual boundary can otherwise change exact integer scores.
        let x1 = first_hit.q_t as f32;
        let x2 = last_hit.q_t as f32;
        let slope = (y2 - y1) as f32 / (x2 - x1);
        let offset = -x1 * slope + y1 as f32;
        let time_factor = 1.0_f32 / (1.0 - slope);
        let pitch_factor = bin_to_hz(first_hit.ref_f) as f32 / bin_to_hz(first_hit.q_f) as f32;
        if !(time_factor > MIN_TIME_FACTOR as f32
            && time_factor < MAX_TIME_FACTOR as f32
            && pitch_factor > MIN_FREQUENCY_FACTOR as f32
            && pitch_factor < MAX_FREQUENCY_FACTOR as f32)
        {
            return Ok(None);
        }

        let filtered: Vec<Hit> = hits
            .iter()
            .copied()
            .filter(|hit| {
                let predicted = slope * hit.q_t as f32 + offset;
                (hit.delta() as f32 - predicted).abs() <= crate::config::QUERY_RANGE as f32
            })
            .collect();
        if filtered.len() <= QUERY_PROFILE.min_hits_filtered_exclusive {
            return Ok(None);
        }
        let first = filtered.first().expect("nonempty filtered hits");
        let last = filtered.last().expect("nonempty filtered hits");
        let q_start = bins_to_seconds(first.q_t);
        let q_stop = bins_to_seconds(last.q_t);
        if q_stop - q_start < QUERY_PROFILE.min_match_duration_seconds {
            return Ok(None);
        }
        let ref_start = bins_to_seconds(first.ref_t);
        let ref_stop = bins_to_seconds(last.ref_t);
        let matching_seconds = (ref_stop - ref_start).ceil();
        if matching_seconds <= 0.0 {
            return Ok(None);
        }
        let mut density = BTreeMap::<u32, usize>::new();
        for hit in &filtered {
            let bin = (bins_to_seconds(hit.ref_t) - ref_start) as u32;
            *density.entry(bin).or_default() += 1;
        }
        let sec_with_match = 1.0 - (matching_seconds - density.len() as f64) / matching_seconds;
        if sec_with_match < MIN_SECONDS_WITH_MATCH {
            return Ok(None);
        }
        let resource = self.store.resource_by_id(resource_id)?;
        let evidence = include_evidence.then(|| make_evidence(&filtered, &density));
        Ok(Some(MatchRow {
            ref_key: resource.key.clone(),
            q_start,
            q_stop,
            ref_start,
            ref_stop,
            score: filtered.len(),
            time_factor: f64::from(time_factor),
            pitch_factor: f64::from(pitch_factor),
            sec_with_match,
            evidence,
        }))
    }
}

/// One distinct probe hash: Panako keeps the last print's (t, f), queues every copy, and orders
/// equal-bucket hashes by first occurrence.
struct ProbeHash {
    last: Fingerprint,
    copies: usize,
    first: usize,
}

/// Voters per Rayon task: one vote is microseconds, so per-resource tasks would mostly measure
/// scheduling.
const VOTE_CHUNK: usize = 64;

/// Gathered hits: `spans` are (resource id, len), consecutive runs of `hits`, ascending by id.
struct Groups {
    hits: Vec<Hit>,
    spans: Vec<(u32, usize)>,
}

impl Groups {
    fn split(&mut self) -> Vec<(u32, &mut [Hit])> {
        let mut rest = &mut self.hits[..];
        let mut parts = Vec::with_capacity(self.spans.len());
        for &(id, len) in &self.spans {
            let (part, tail) = rest.split_at_mut(len);
            parts.push((id, part));
            rest = tail;
        }
        parts
    }
}

/// The most common time offset, ties broken as Java's HashMap iteration would break them.
fn modal_delta(hits: &[Hit]) -> i64 {
    let mut deltas: Vec<(i64, usize)> = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| (hit.delta(), index))
        .collect();
    deltas.sort_unstable();
    // One (delta, count, first index) per distinct offset.
    let mut runs: Vec<(i64, usize, usize)> = Vec::new();
    for &(delta, index) in &deltas {
        match runs.last_mut() {
            Some(run) if run.0 == delta => run.1 += 1,
            _ => runs.push((delta, 1, index)),
        }
    }
    let capacity = java_hash_map_capacity(runs.len());
    runs.into_iter()
        .max_by(|(delta_a, count_a, first_a), (delta_b, count_b, first_b)| {
            count_a
                .cmp(count_b)
                .then_with(|| java_bucket(*delta_b, capacity).cmp(&java_bucket(*delta_a, capacity)))
                .then_with(|| first_b.cmp(first_a))
        })
        .map(|(delta, _, _)| delta)
        .expect("modal vote requires hits")
}

fn java_hash_map_capacity(entries: usize) -> usize {
    let mut capacity = 16;
    while entries > capacity * 3 / 4 {
        capacity *= 2;
    }
    capacity
}

fn java_bucket(delta: i64, capacity: usize) -> usize {
    let hash = delta as i32 as u32;
    ((hash ^ (hash >> 16)) as usize) & (capacity - 1)
}

fn java_long_bucket(hash: u64, capacity: usize) -> usize {
    let folded = (hash ^ (hash >> 32)) as u32;
    ((folded ^ (folded >> 16)) as usize) & (capacity - 1)
}

fn make_evidence(filtered: &[Hit], density: &BTreeMap<u32, usize>) -> Evidence {
    let mut offsets = HashMap::<i64, usize>::new();
    for &hit in filtered {
        *offsets.entry(hit.delta()).or_default() += 1;
    }
    let mut offset_top: Vec<_> = offsets
        .into_iter()
        .map(|(bin, count)| HistogramBin { bin, count })
        .collect();
    offset_top.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.bin.cmp(&b.bin)));
    offset_top.truncate(10);
    Evidence {
        hits: filtered
            .iter()
            .map(|hit| EvidenceHit {
                q_t: hit.q_t,
                ref_t: hit.ref_t,
                q_seconds: bins_to_seconds(hit.q_t),
                ref_seconds: bins_to_seconds(hit.ref_t),
                original_hash: hit.original_hash,
                matched_hash: hit.matched_hash,
            })
            .collect(),
        offset_top,
        per_second: density
            .iter()
            .map(|(&second, &count)| DensityBin { second, count })
            .collect(),
    }
}

#[cfg(test)]
#[path = "matcher_reference.rs"]
mod reference;

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(q_t: u32, ref_t: u32) -> Hit {
        Hit {
            ref_t,
            ref_f: 100,
            q_t,
            q_f: 100,
            original_hash: 1,
            matched_hash: 1,
        }
    }

    #[test]
    fn modal_vote_matches_java_hash_map_tie_order() {
        // Integer 20 lands in bucket 4 and 10 in bucket 10 of Java's initial table.
        assert_eq!(modal_delta(&[hit(0, 10), hit(0, 20)]), 20);
    }

    use std::path::{Path, PathBuf};

    use crate::{DumpResource, ResourceMeta, load_dump_dir};

    /// A tiny deterministic generator: the test must not depend on a random crate's stream.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn dump(key: &str, prints: Vec<Fingerprint>) -> DumpResource {
        DumpResource {
            meta: ResourceMeta {
                source_id: "1".into(),
                key: key.into(),
                duration: 300.0,
                declared_prints: prints.len() as u64,
            },
            prints,
            prints_path: PathBuf::new(),
        }
    }

    /// Crowded on purpose: a small hash space makes the +-2 neighbourhoods long and shared,
    /// resources reuse each other's material at new offsets (several lines per pair), and some
    /// resources repeat a hash at one time with different bins.
    fn crowded_store(root: &Path, rng: &mut Rng) -> (Store, Vec<Vec<Fingerprint>>) {
        let mut sources: Vec<Vec<Fingerprint>> = Vec::new();
        for index in 0..48 {
            let mut prints = Vec::new();
            let mut t = rng.below(50) as u32;
            for _ in 0..(300 + rng.below(700)) {
                t += 1 + rng.below(12) as u32;
                let hash = rng.below(6_000);
                let f = 100 + rng.below(300) as u16;
                prints.push(Fingerprint::new(hash, t, f));
                if rng.below(40) == 0 {
                    prints.push(Fingerprint::new(hash, t, f + 1));
                }
            }
            if index >= 8 {
                // Borrow a stretch of an earlier source at a new offset (a shared passage).
                let donor = &sources[rng.below(index as u64) as usize];
                let from = rng.below(donor.len() as u64 / 2) as usize;
                let shift = 2_000 + rng.below(3_000) as u32;
                for print in &donor[from..(from + 200).min(donor.len())] {
                    prints.push(Fingerprint::new(print.hash, print.t + shift, print.f));
                }
            }
            sources.push(prints);
        }
        let resources = sources
            .iter()
            .enumerate()
            .map(|(index, prints)| dump(&format!("r{index:02}"), prints.clone()))
            .collect();
        (
            Store::build(root, resources).expect("crowded store"),
            sources,
        )
    }

    /// Probes: excerpts at an offset, with repeated hashes (Panako's last-print quirk and copy
    /// multiplicity), slight time stretch and unrelated noise.
    fn probes(sources: &[Vec<Fingerprint>], rng: &mut Rng, count: usize) -> Vec<Vec<Fingerprint>> {
        (0..count)
            .map(|_| {
                let source = &sources[rng.below(sources.len() as u64) as usize];
                let from = rng.below(source.len() as u64 / 2) as usize;
                let to = (from + 100 + rng.below(600) as usize).min(source.len());
                let stretch = [1.0, 1.0, 1.03, 0.97][rng.below(4) as usize];
                let mut prints: Vec<Fingerprint> = source[from..to]
                    .iter()
                    .map(|print| {
                        let t = (f64::from(print.t) * stretch) as u32 + 7;
                        Fingerprint::new(print.hash, t, print.f)
                    })
                    .collect();
                for _ in 0..rng.below(80) {
                    let copy = prints[rng.below(prints.len() as u64) as usize];
                    prints.push(Fingerprint::new(
                        copy.hash,
                        copy.t + rng.below(400) as u32,
                        copy.f,
                    ));
                }
                for _ in 0..rng.below(120) {
                    prints.push(Fingerprint::new(
                        rng.below(6_000),
                        rng.below(5_000) as u32,
                        100 + rng.below(300) as u16,
                    ));
                }
                prints
            })
            .collect()
    }

    fn json<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializable")
    }

    /// Every public question the matcher answers, through the optimized path and the reference
    /// path, must serialize identically: scores, lines, factors, evidence hit order and all.
    fn assert_same_answers(store: &Store, probes: &[Vec<Fingerprint>]) {
        let keys: Vec<&str> = store.resources().iter().map(|r| r.key.as_str()).collect();
        let skipping = Matcher::skipping(store, &keys[..3]);
        let plain = Matcher::new(store);
        let mut compared_rows = 0;
        for (index, prints) in probes.iter().enumerate() {
            let hashes: Vec<u64> = prints.iter().map(|print| print.hash).collect();
            let full = store.lookup_table(&hashes).unwrap();
            // A targets-only table (every third resource) keeps store-wide hash presence.
            let targets: Vec<u32> = store.resources().iter().step_by(3).map(|r| r.id).collect();
            let among = store.lookup_table_among(&hashes, &targets).unwrap();
            for matcher in [&plain, &skipping] {
                for multi_line in [false, true] {
                    for (k, evidence) in [(1, false), (5, true), (100, true), (100, false)] {
                        for table in [None, Some(&full), Some(&among)] {
                            let fast = matcher
                                .match_restricted_with(prints, k, evidence, multi_line, None, table)
                                .unwrap();
                            let reference = matcher
                                .reference_match(prints, k, evidence, multi_line, None, table)
                                .unwrap();
                            assert_eq!(
                                json(&fast),
                                json(&reference),
                                "probe {index}, multi_line {multi_line}, k {k}"
                            );
                            compared_rows += fast.len();
                        }
                    }
                }
            }
            let target = store.resource(keys[index % keys.len()]).unwrap().id;
            for multi_line in [false, true] {
                let fast = plain
                    .match_restricted_with(prints, 5, true, multi_line, Some(target), None)
                    .unwrap();
                let reference = plain
                    .reference_match(prints, 5, true, multi_line, Some(target), None)
                    .unwrap();
                assert_eq!(json(&fast), json(&reference), "probe {index} pair");
            }
        }
        // The comparison must have exercised real matches, not agreed on empty answers.
        assert!(
            compared_rows > probes.len() * 20,
            "only {compared_rows} rows"
        );
    }

    #[test]
    fn lean_gather_equals_reference_on_a_crowded_store() {
        let root = tempfile::tempdir().unwrap();
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (store, sources) = crowded_store(root.path(), &mut rng);
        let probes = probes(&sources, &mut rng, 60);
        assert_same_answers(&store, &probes);
    }

    #[test]
    fn lean_gather_equals_reference_on_fixture_prints() {
        let root = tempfile::tempdir().unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
        let resources = load_dump_dir(&dir).expect("fixture dump");
        let sources: Vec<Vec<Fingerprint>> = resources.iter().map(|r| r.prints.clone()).collect();
        let store = Store::build(root.path(), resources).expect("fixture store");
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let probes = probes(&sources, &mut rng, 16);
        assert_same_answers(&store, &probes);
    }

    /// Hashes present only in non-target resources still size Panako's hash map, and the map's
    /// capacity orders a target's equal-time hits. Built so that it matters: T holds two hashes
    /// per time whose bucket order flips between capacity 64 and 128 (low bits 65 vs 2), and the
    /// probe adds 20 hashes found only in R, lifting 40 present hashes to 60.
    #[test]
    fn presence_only_hashes_order_a_targets_hits() {
        let root = tempfile::tempdir().unwrap();
        let mut target = Vec::new();
        for index in 0..20_u64 {
            let t = 100 * index as u32;
            let base = 128 * (10 * index + 10);
            target.push(Fingerprint::new(base + 65, t, 200));
            target.push(Fingerprint::new(base + 2, t, 200));
        }
        let elsewhere: Vec<Fingerprint> = (0..20_u64)
            .map(|index| Fingerprint::new(128 * (10 * index + 500) + 30, 50 * index as u32, 150))
            .collect();
        let store = Store::build(
            root.path(),
            vec![dump("T", target.clone()), dump("R", elsewhere.clone())],
        )
        .unwrap();
        let t_id = store.resource("T").unwrap().id;
        let matcher = Matcher::new(&store);
        let answer = |prints: &[Fingerprint]| {
            let hashes: Vec<u64> = prints.iter().map(|print| print.hash).collect();
            let among = store.lookup_table_among(&hashes, &[t_id]).unwrap();
            let fast = matcher
                .match_restricted_with(prints, 5, true, false, None, Some(&among))
                .unwrap();
            let reference = matcher
                .reference_match(prints, 5, true, false, None, Some(&among))
                .unwrap();
            assert_eq!(json(&fast), json(&reference));
            assert_eq!(fast.len(), 1, "T must match");
            json(&fast)
        };
        let mut with_elsewhere = target.clone();
        with_elsewhere.extend(elsewhere);
        // The R-only hashes change T's evidence order: the case is live, not vacuous.
        assert_ne!(answer(&target), answer(&with_elsewhere));
    }

    #[test]
    fn modal_vote_matches_reference_on_ties() {
        let mut rng = Rng(7);
        for _ in 0..2_000 {
            let hits: Vec<Hit> = (0..1 + rng.below(40))
                .map(|_| hit(rng.below(30) as u32, rng.below(30) as u32))
                .collect();
            assert_eq!(modal_delta(&hits), reference::reference_modal_delta(&hits));
        }
    }

    #[test]
    fn modal_vote_handles_single_hit() {
        assert_eq!(modal_delta(&[hit(7, 10)]), 3);
    }
}
