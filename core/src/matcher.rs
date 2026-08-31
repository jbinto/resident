use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::{
    HIT_PART_DIVIDER, HIT_PART_MAX_SIZE, MAX_FREQUENCY_FACTOR, MAX_TIME_FACTOR,
    MIN_FREQUENCY_FACTOR, MIN_HITS_FILTERED_EXCLUSIVE, MIN_HITS_UNFILTERED,
    MIN_MATCH_DURATION_SECONDS, MIN_SECONDS_WITH_MATCH, MIN_TIME_FACTOR, bin_to_hz,
    bins_to_seconds,
};
use crate::store::StoredHit;
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
}

impl<'a> Matcher<'a> {
    pub const fn new(store: &'a Store) -> Self {
        Self { store }
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
        if prints.is_empty() {
            return Err(Error::BadRequest("probe prints must not be empty".into()));
        }
        if k == 0 {
            return Err(Error::BadRequest("k must be greater than zero".into()));
        }

        // Panako queues every input hash but its hash-keyed probe map retains the last (t,f)
        // for duplicates. Preserve that compatibility quirk inside this module only.
        let mut last_print = HashMap::new();
        let mut multiplicity = HashMap::<u64, usize>::new();
        let mut first_occurrence = HashMap::<u64, usize>::new();
        for (index, &print) in prints.iter().enumerate() {
            last_print.insert(print.hash, print);
            *multiplicity.entry(print.hash).or_default() += 1;
            first_occurrence.entry(print.hash).or_insert(index);
        }
        let mut hashes: Vec<_> = multiplicity.keys().copied().collect();
        hashes.sort_unstable();

        let lookups: Vec<Result<(u64, Vec<StoredHit>)>> = hashes
            .par_iter()
            .map(|&hash| {
                only_resource
                    .map_or_else(
                        || self.store.lookup(hash),
                        |resource_id| self.store.lookup_resource(hash, resource_id),
                    )
                    .map(|hits| (hash, hits))
            })
            .collect();
        let mut lookups: Vec<_> = lookups.into_iter().collect::<Result<Vec<_>>>()?;
        lookups.retain(|(_, hits)| !hits.is_empty());
        let capacity = java_hash_map_capacity(lookups.len());
        lookups
            .sort_by_key(|(hash, _)| (java_long_bucket(*hash, capacity), first_occurrence[hash]));
        let mut by_resource = HashMap::<u32, Vec<Hit>>::new();
        for (hash, stored_hits) in lookups {
            let query = last_print[&hash];
            let copies = multiplicity[&hash];
            for stored in stored_hits {
                if only_resource.is_some_and(|id| id != stored.resource_id) {
                    continue;
                }
                let hit = Hit {
                    ref_t: stored.t,
                    ref_f: stored.f,
                    q_t: query.t,
                    q_f: query.f,
                    original_hash: hash,
                    matched_hash: stored.matched_hash,
                };
                by_resource
                    .entry(stored.resource_id)
                    .or_default()
                    .extend(std::iter::repeat_n(hit, copies));
            }
        }

        let mut rows = Vec::new();
        if multi_line {
            let candidates: Vec<_> = by_resource
                .into_par_iter()
                .filter_map(|(resource_id, hits)| {
                    (hits.len() >= MIN_HITS_UNFILTERED).then_some((resource_id, hits))
                })
                .map(|(resource_id, hits)| self.vote_lines(resource_id, hits, k, include_evidence))
                .collect();
            for candidate in candidates {
                rows.extend(candidate?);
            }
        } else {
            let candidates: Vec<_> = by_resource
                .into_par_iter()
                .filter_map(|(resource_id, hits)| {
                    (hits.len() >= MIN_HITS_UNFILTERED).then_some((resource_id, hits))
                })
                .map(|(resource_id, hits)| self.vote(resource_id, hits, include_evidence))
                .collect();
            for candidate in candidates {
                if let Some(row) = candidate? {
                    rows.push(row);
                }
            }
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

    fn vote_lines(
        &self,
        resource_id: u32,
        mut remaining: Vec<Hit>,
        limit: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        let mut rows = Vec::new();
        while remaining.len() >= MIN_HITS_UNFILTERED && rows.len() < limit {
            let Some(mut row) = self.vote(resource_id, remaining.clone(), true)? else {
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
            remaining.retain(|hit| {
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
            if include_evidence {
                row.evidence = Some(evidence);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn vote(
        &self,
        resource_id: u32,
        mut hits: Vec<Hit>,
        include_evidence: bool,
    ) -> Result<Option<MatchRow>> {
        hits.sort_by_key(|hit| hit.q_t);
        let part_len =
            HIT_PART_MAX_SIZE.min(MIN_HITS_UNFILTERED.max(hits.len() / HIT_PART_DIVIDER));
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

        let filtered: Vec<_> = hits
            .into_iter()
            .filter(|hit| {
                let predicted = slope * hit.q_t as f32 + offset;
                (hit.delta() as f32 - predicted).abs() <= crate::config::QUERY_RANGE as f32
            })
            .collect();
        if filtered.len() <= MIN_HITS_FILTERED_EXCLUSIVE {
            return Ok(None);
        }
        let first = filtered.first().expect("nonempty filtered hits");
        let last = filtered.last().expect("nonempty filtered hits");
        let q_start = bins_to_seconds(first.q_t);
        let q_stop = bins_to_seconds(last.q_t);
        if q_stop - q_start < MIN_MATCH_DURATION_SECONDS {
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

fn modal_delta(hits: &[Hit]) -> i64 {
    let mut counts = HashMap::<i64, (usize, usize)>::new();
    for (index, &hit) in hits.iter().enumerate() {
        let value = counts.entry(hit.delta()).or_insert((0, index));
        value.0 += 1;
    }
    let capacity = java_hash_map_capacity(counts.len());
    counts
        .into_iter()
        .max_by(
            |(delta_a, (count_a, first_a)), (delta_b, (count_b, first_b))| {
                count_a
                    .cmp(count_b)
                    .then_with(|| {
                        java_bucket(*delta_b, capacity).cmp(&java_bucket(*delta_a, capacity))
                    })
                    .then_with(|| first_b.cmp(first_a))
            },
        )
        .map(|(delta, _)| delta)
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

    #[test]
    fn modal_vote_handles_single_hit() {
        assert_eq!(modal_delta(&[hit(7, 10)]), 3);
    }
}
