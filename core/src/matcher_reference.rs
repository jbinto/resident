//! The straightforward matcher this crate shipped before LEAN-GATHER, kept verbatim as the
//! equivalence oracle for the optimized path (AGENTS.md: optimizations on compatibility paths need
//! equivalence against the reference path). Test-only.

use std::collections::{BTreeMap, HashMap};

use rayon::prelude::*;

use super::{
    Hit, MatchRow, Matcher, java_bucket, java_hash_map_capacity, java_long_bucket, make_evidence,
};
use crate::config::{
    HIT_PART_DIVIDER, HIT_PART_MAX_SIZE, MAX_FREQUENCY_FACTOR, MAX_TIME_FACTOR,
    MIN_FREQUENCY_FACTOR, MIN_SECONDS_WITH_MATCH, MIN_TIME_FACTOR, QUERY_PROFILE, bin_to_hz,
    bins_to_seconds,
};
use crate::store::{LookupTable, StoredHit};
use crate::{Error, Fingerprint, Result};

impl Matcher<'_> {
    pub(super) fn reference_match(
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

        let lookups: Vec<Result<(u64, Vec<StoredHit>, bool)>> = hashes
            .par_iter()
            .map(|&hash| {
                let hits = only_resource.map_or_else(
                    || match table.and_then(|table| table.get(hash)) {
                        Some(hits) => Ok(hits.to_vec()),
                        None => self.store.lookup(hash),
                    },
                    |resource_id| self.store.lookup_resource(hash, resource_id),
                )?;
                let global_present = if only_resource.is_some() {
                    self.store.has_match(hash)?
                } else {
                    !hits.is_empty()
                };
                Ok((hash, hits, global_present))
            })
            .collect();
        let mut lookups: Vec<_> = lookups.into_iter().collect::<Result<Vec<_>>>()?;
        lookups.retain(|(_, _, global_present)| *global_present);
        let capacity = java_hash_map_capacity(lookups.len());
        lookups.sort_by_key(|(hash, _, _)| {
            (java_long_bucket(*hash, capacity), first_occurrence[hash])
        });
        let mut by_resource = HashMap::<u32, Vec<Hit>>::new();
        for (hash, stored_hits, _) in lookups {
            let query = last_print[&hash];
            let copies = multiplicity[&hash];
            for stored in stored_hits {
                if only_resource.is_some_and(|id| id != stored.resource_id)
                    || self.skip.binary_search(&stored.resource_id).is_ok()
                {
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
                    (hits.len() >= QUERY_PROFILE.min_hits_unfiltered).then_some((resource_id, hits))
                })
                .map(|(resource_id, hits)| {
                    self.reference_vote_lines(resource_id, hits, k, include_evidence)
                })
                .collect();
            for candidate in candidates {
                rows.extend(candidate?);
            }
        } else {
            let candidates: Vec<_> = by_resource
                .into_par_iter()
                .filter_map(|(resource_id, hits)| {
                    (hits.len() >= QUERY_PROFILE.min_hits_unfiltered).then_some((resource_id, hits))
                })
                .map(|(resource_id, hits)| self.reference_vote(resource_id, hits, include_evidence))
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

    fn reference_vote_lines(
        &self,
        resource_id: u32,
        mut remaining: Vec<Hit>,
        limit: usize,
        include_evidence: bool,
    ) -> Result<Vec<MatchRow>> {
        let mut rows = Vec::new();
        while remaining.len() >= QUERY_PROFILE.min_hits_unfiltered && rows.len() < limit {
            let Some(mut row) = self.reference_vote(resource_id, remaining.clone(), true)? else {
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

    fn reference_vote(
        &self,
        resource_id: u32,
        mut hits: Vec<Hit>,
        include_evidence: bool,
    ) -> Result<Option<MatchRow>> {
        hits.sort_by_key(|hit| hit.q_t);
        let part_len = HIT_PART_MAX_SIZE.min(
            QUERY_PROFILE
                .min_hits_unfiltered
                .max(hits.len() / HIT_PART_DIVIDER),
        );
        let first = &hits[..part_len];
        let last = &hits[hits.len() - part_len..];
        let y1 = reference_modal_delta(first);
        let y2 = reference_modal_delta(last);
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

pub(super) fn reference_modal_delta(hits: &[Hit]) -> i64 {
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
