//! CANDIDATES: which stored references a probe contains, and how much of each side the match
//! covers. A geometric summary over the unchanged multiline voter; naming and judgement stay with
//! the caller (docs/SCOPE.md).
//!
//! Coverage comes two ways. Span coverage is the union of the lines' first-to-last-hit spans: a
//! sparse line of a few dozen hits can span most of a short window. Supported coverage counts only
//! seconds inside dense runs of hits, the passages' support rule (consecutive hits at most 1.5 s
//! apart on both clocks), so a sparse line covers little. `relation` reads supported coverage.

use std::collections::BTreeMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::TIME_BIN_SECONDS;
use crate::passage::support_from_hits;
use crate::{Error, Fingerprint, MatchRow, Matcher, Result, Store, SupportSpan};

/// Probe regions, as `crosscheck`'s: each votes independently, so one reference can match at
/// several offsets and a long probe can hold several references.
const REGION_SECONDS: f64 = 30.0;
/// A side counts as covered once this fraction of its duration is supported (dense runs of hits).
/// Measured on a 3,984-file library: same-title duplicates reach 0.58-0.99, a vocal against its own
/// instrumental or a cappella at most 0.47 (span coverage put many of those at 0.7-0.9).
pub const COVERED_FRACTION: f64 = 0.5;

/// Which side of the match is (mostly) covered: geometry, not a verdict on identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Both sides covered: the same recording, start to end.
    Full,
    /// The probe is covered but the reference is not: the probe is an excerpt of it.
    QueryWithinReference,
    /// The reference is covered but the probe is not: the probe contains it.
    ReferenceWithinQuery,
    /// Neither side covered: shared material (an edit, a version, a sample, a short overlap).
    Partial,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub ref_key: String,
    /// Sum of accepted line scores over all regions.
    pub matched_hits: usize,
    pub query_matched_seconds: f64,
    pub reference_matched_seconds: f64,
    /// Matched seconds over the probe duration, and over the reference's stored duration.
    pub query_coverage: f64,
    pub reference_coverage: f64,
    /// Seconds inside dense support runs of the lines' hits, on each clock (passages' rule).
    pub query_supported_seconds: f64,
    pub reference_supported_seconds: f64,
    /// Supported seconds over the probe duration, and over the reference's stored duration.
    pub query_supported_coverage: f64,
    pub reference_supported_coverage: f64,
    /// From the highest-scoring line: reference time minus probe time at its start, in seconds.
    pub offset_seconds: f64,
    pub time_factor: f64,
    pub pitch_factor: f64,
    pub relation: Relation,
}

/// Rank the references `prints` matches, best first (matched hits, then key), at most `k`.
/// `query_duration` is the probe's length in seconds; without it, the prints' own extent is used.
pub fn candidates(
    store: &Store,
    prints: &[Fingerprint],
    query_duration: Option<f64>,
    k: usize,
) -> Result<Vec<Candidate>> {
    ranked(store, prints, query_duration, k, None)
}

/// `candidates` scored against `targets` only (TARGETED-CANDIDATES): a pair check of a probe
/// against references the caller already suspects. Each target's row is the row an unbounded
/// `candidates` gives it, field for field; targets the probe does not match are absent. Only the
/// targets' shards are gathered and voted, so the cost follows the targets, not the store.
pub fn candidates_among(
    store: &Store,
    prints: &[Fingerprint],
    query_duration: Option<f64>,
    targets: &[String],
    k: usize,
) -> Result<Vec<Candidate>> {
    if targets.is_empty() {
        return Err(Error::BadRequest("targets must not be empty".into()));
    }
    let ids = targets
        .iter()
        .map(|key| store.resource(key).map(|resource| resource.id))
        .collect::<Result<Vec<u32>>>()?;
    ranked(store, prints, query_duration, k, Some(&ids))
}

fn ranked(
    store: &Store,
    prints: &[Fingerprint],
    query_duration: Option<f64>,
    k: usize,
    targets: Option<&[u32]>,
) -> Result<Vec<Candidate>> {
    if prints.is_empty() {
        return Err(Error::BadRequest("probe prints must not be empty".into()));
    }
    if k == 0 {
        return Err(Error::BadRequest("k must be greater than zero".into()));
    }
    let t_min = prints.iter().map(|print| print.t).min().expect("nonempty");
    let t_max = prints.iter().map(|print| print.t).max().expect("nonempty");
    let query_duration = query_duration.unwrap_or(f64::from(t_max - t_min + 1) * TIME_BIN_SECONDS);
    if !query_duration.is_finite() || query_duration <= 0.0 {
        return Err(Error::BadRequest("query duration must be positive".into()));
    }

    let region_bins = (REGION_SECONDS / TIME_BIN_SECONDS) as u32;
    let mut regions = BTreeMap::<u32, Vec<Fingerprint>>::new();
    for &print in prints {
        regions
            .entry((print.t - t_min) / region_bins)
            .or_default()
            .push(print);
    }
    let regions: Vec<Vec<Fingerprint>> = regions.into_values().collect();
    let hashes: Vec<u64> = regions.iter().flatten().map(|print| print.hash).collect();
    // A target-only table keeps the store-wide presence of every hash, which orders the voter's
    // hits (Panako's hash-map capacity), so the targets' rows are unchanged.
    let table = match targets {
        Some(ids) => store.lookup_table_among(&hashes, ids)?,
        None => store.lookup_table(&hashes)?,
    };
    let matcher = Matcher::new(store);
    let all = store.resources().len().max(1);
    let rows: Vec<Result<Vec<MatchRow>>> = regions
        .par_iter()
        .filter(|prints| prints.len() >= 2)
        .map(|prints| matcher.match_prints_multiline_with(prints, all, true, &table))
        .collect();

    let mut by_key = BTreeMap::<String, Vec<MatchRow>>::new();
    for region in rows {
        for row in region? {
            by_key.entry(row.ref_key.clone()).or_default().push(row);
        }
    }
    let mut found = Vec::with_capacity(by_key.len());
    for (ref_key, lines) in by_key {
        // Duration metadata can be wrong (Panako's cached-store constant); a reference lasts at
        // least until its last stored print.
        let resource = store.resource(&ref_key)?;
        let reference_duration = resource
            .duration
            .max(crate::config::bins_to_seconds(resource.t_max));
        let query_matched_seconds =
            union_seconds(lines.iter().map(|line| (line.q_start, line.q_stop)));
        let reference_matched_seconds =
            union_seconds(lines.iter().map(|line| (line.ref_start, line.ref_stop)));
        let support: Vec<SupportSpan> = lines
            .iter()
            .flat_map(|line| {
                let hits = line.evidence.as_ref().map_or(&[][..], |e| &e.hits[..]);
                support_from_hits(hits)
            })
            .collect();
        let query_supported_seconds =
            union_seconds(support.iter().map(|span| (span.a_start, span.a_stop)));
        let reference_supported_seconds =
            union_seconds(support.iter().map(|span| (span.b_start, span.b_stop)));
        let query_coverage = (query_matched_seconds / query_duration).min(1.0);
        let reference_coverage = fraction(reference_matched_seconds, reference_duration);
        let query_supported_coverage = (query_supported_seconds / query_duration).min(1.0);
        let reference_supported_coverage =
            fraction(reference_supported_seconds, reference_duration);
        let best = lines
            .iter()
            .max_by(|a, b| {
                a.score
                    .cmp(&b.score)
                    .then_with(|| b.q_start.total_cmp(&a.q_start))
            })
            .expect("a reference has at least one line");
        let relation = match (
            query_supported_coverage >= COVERED_FRACTION,
            reference_supported_coverage >= COVERED_FRACTION,
        ) {
            (true, true) => Relation::Full,
            (true, false) => Relation::QueryWithinReference,
            (false, true) => Relation::ReferenceWithinQuery,
            (false, false) => Relation::Partial,
        };
        found.push(Candidate {
            matched_hits: lines.iter().map(|line| line.score).sum(),
            offset_seconds: best.ref_start - best.q_start,
            time_factor: best.time_factor,
            pitch_factor: best.pitch_factor,
            ref_key,
            query_matched_seconds,
            reference_matched_seconds,
            query_coverage,
            reference_coverage,
            query_supported_seconds,
            reference_supported_seconds,
            query_supported_coverage,
            reference_supported_coverage,
            relation,
        });
    }
    found.sort_by(|a, b| {
        b.matched_hits
            .cmp(&a.matched_hits)
            .then_with(|| a.ref_key.cmp(&b.ref_key))
    });
    found.truncate(k);
    Ok(found)
}

/// Cut a probe to `[start, stop)` seconds of its own clock. Returns the prints inside and the
/// probe duration to measure coverage against: the window's length, else `whole_duration`.
pub fn probe_window(
    prints: Vec<Fingerprint>,
    window: Option<(f64, f64)>,
    whole_duration: Option<f64>,
) -> Result<(Vec<Fingerprint>, Option<f64>)> {
    let Some((start, stop)) = window else {
        return Ok((prints, whole_duration));
    };
    if !start.is_finite() || !stop.is_finite() || start < 0.0 || start >= stop {
        return Err(Error::BadRequest(
            "window must contain finite, non-negative increasing seconds".into(),
        ));
    }
    let from = crate::config::seconds_to_bin(start).unwrap_or(0);
    let to = crate::config::seconds_to_bin(stop).unwrap_or(u32::MAX);
    let inside: Vec<_> = prints
        .into_iter()
        .filter(|print| print.t >= from && print.t < to)
        .collect();
    if inside.is_empty() {
        return Err(Error::BadRequest("window contains no fingerprints".into()));
    }
    Ok((inside, Some(stop - start)))
}

fn fraction(seconds: f64, duration: f64) -> f64 {
    if duration > 0.0 {
        (seconds / duration).min(1.0)
    } else {
        0.0
    }
}

/// Total length of the union of `[start, stop]` intervals.
fn union_seconds(intervals: impl Iterator<Item = (f64, f64)>) -> f64 {
    let mut intervals: Vec<_> = intervals.collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (start, stop) in intervals {
        match current {
            Some((_, end)) if start <= end => {
                current = current.map(|(s, e)| (s, e.max(stop)));
            }
            _ => {
                if let Some((s, e)) = current {
                    total += e - s;
                }
                current = Some((start, stop));
            }
        }
    }
    if let Some((s, e)) = current {
        total += e - s;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_merges_overlaps_and_keeps_gaps() {
        let total = union_seconds([(0.0, 10.0), (5.0, 12.0), (20.0, 25.0)].into_iter());
        assert!((total - 17.0).abs() < 1e-9);
        assert_eq!(union_seconds(std::iter::empty()), 0.0);
    }
}
