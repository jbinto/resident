use std::collections::{BTreeMap, HashSet};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::{TIME_BIN_SECONDS, seconds_to_bin};
use crate::{Error, Evidence, Matcher, Result, Store};

const REGION_SECONDS: f64 = 30.0;

#[derive(Clone, Copy)]
pub(crate) struct RegionGeometry {
    pub window_seconds: f64,
    pub hop_seconds: f64,
    pub anchor_at_zero: bool,
}

const COMPATIBILITY_GEOMETRY: RegionGeometry = RegionGeometry {
    window_seconds: REGION_SECONDS,
    hop_seconds: REGION_SECONDS,
    anchor_at_zero: false,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub a_start: f64,
    pub a_stop: f64,
    pub b_start: f64,
    pub b_stop: f64,
    pub score: usize,
    pub time_factor: f64,
    pub pitch_factor: f64,
    pub sec_with_match: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrosscheckMatch {
    pub ref_key: String,
    pub segments: Vec<Segment>,
    pub score_total: usize,
}

pub fn span(
    store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    evidence: bool,
) -> Result<Vec<Segment>> {
    span_between(store, store, a_key, a_window, b_key, evidence)
}

pub fn span_between(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    evidence: bool,
) -> Result<Vec<Segment>> {
    ensure_compatible(a_store, b_store)?;
    b_store.resource(b_key)?;
    let regions = region_prints(a_store, a_key, a_window, COMPATIBILITY_GEOMETRY)?;
    let matcher = Matcher::new(b_store);
    let candidates: Vec<_> = regions
        .par_iter()
        .map(|prints| matcher.match_resource(prints, b_key, evidence))
        .collect();
    let mut segments = Vec::new();
    for candidate in candidates {
        if let Some(row) = candidate? {
            segments.push(row.into());
        }
    }
    stable_segments(&mut segments);
    Ok(segments)
}

pub fn span_multiline(
    store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    evidence: bool,
) -> Result<Vec<Segment>> {
    span_between_multiline(store, store, a_key, a_window, b_key, evidence)
}

pub fn span_between_multiline(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    evidence: bool,
) -> Result<Vec<Segment>> {
    span_between_multiline_geometry(
        a_store,
        b_store,
        a_key,
        a_window,
        b_key,
        evidence,
        COMPATIBILITY_GEOMETRY,
    )
}

pub(crate) fn span_between_multiline_geometry(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    evidence: bool,
    geometry: RegionGeometry,
) -> Result<Vec<Segment>> {
    ensure_compatible(a_store, b_store)?;
    b_store.resource(b_key)?;
    let regions = region_prints(a_store, a_key, a_window, geometry)?;
    let matcher = Matcher::new(b_store);
    let rows: Vec<_> = regions
        .par_iter()
        .map(|prints| matcher.match_resource_multiline(prints, b_key, usize::MAX, evidence))
        .collect();
    let mut segments = Vec::new();
    // Indexed parallel collection retains region order; Matcher ranks each region's lines.
    for rows in rows {
        segments.extend(rows?.into_iter().map(Segment::from));
    }
    Ok(segments)
}

pub fn crosscheck(
    store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    k: usize,
    evidence: bool,
    skip_self: bool,
) -> Result<Vec<CrosscheckMatch>> {
    crosscheck_between(
        store, store, a_key, a_window, targets, k, evidence, skip_self,
    )
}

/// `skip_self` never votes B's resource named `a_key` (if B has one), so a self-check spends no
/// time on its own diagonal and `k` counts only other references. Every other reference's
/// segments are identical to the unskipped answer.
#[allow(clippy::too_many_arguments)]
pub fn crosscheck_between(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    k: usize,
    evidence: bool,
    skip_self: bool,
) -> Result<Vec<CrosscheckMatch>> {
    if k == 0 {
        return Err(Error::BadRequest("k must be greater than zero".into()));
    }
    ensure_compatible(a_store, b_store)?;
    let target_ids = if let Some(keys) = targets {
        let mut ids = HashSet::new();
        for key in keys {
            ids.insert(b_store.resource(key)?.id);
        }
        Some(ids)
    } else {
        None
    };
    let regions = region_prints(a_store, a_key, a_window, COMPATIBILITY_GEOMETRY)?;
    let skip: &[&str] = if skip_self { &[a_key] } else { &[] };
    let matcher = Matcher::skipping(b_store, skip);
    // One sorted walk per shard for every distinct hash (BATCH-LOOKUP), not 64 binary searches
    // per hash per region: the same hits in the same order.
    let all_hashes: Vec<u64> = regions.iter().flatten().map(|print| print.hash).collect();
    let table = b_store.lookup_table(&all_hashes)?;
    let rows: Vec<_> = regions
        .par_iter()
        .map(|prints| {
            matcher.match_prints_with(prints, b_store.resources().len().max(1), evidence, &table)
        })
        .collect();
    let mut by_key = BTreeMap::<String, Vec<Segment>>::new();
    for rows in rows {
        for row in rows? {
            let resource = b_store.resource(&row.ref_key)?;
            if target_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&resource.id))
            {
                continue;
            }
            by_key
                .entry(row.ref_key.clone())
                .or_default()
                .push(row.into());
        }
    }
    let mut matches: Vec<_> = by_key
        .into_iter()
        .map(|(ref_key, mut segments)| {
            stable_segments(&mut segments);
            CrosscheckMatch {
                ref_key,
                score_total: segments.iter().map(|segment| segment.score).sum(),
                segments,
            }
        })
        .collect();
    matches.sort_by(|a, b| {
        b.score_total
            .cmp(&a.score_total)
            .then_with(|| a.ref_key.cmp(&b.ref_key))
    });
    matches.truncate(k);
    Ok(matches)
}

pub fn crosscheck_multiline(
    store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    k: usize,
    evidence: bool,
    skip_self: bool,
) -> Result<Vec<CrosscheckMatch>> {
    crosscheck_between_multiline(
        store, store, a_key, a_window, targets, k, evidence, skip_self,
    )
}

/// `skip_self` as in `crosscheck_between`.
#[allow(clippy::too_many_arguments)]
pub fn crosscheck_between_multiline(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    k: usize,
    evidence: bool,
    skip_self: bool,
) -> Result<Vec<CrosscheckMatch>> {
    if k == 0 {
        return Err(Error::BadRequest("k must be greater than zero".into()));
    }
    let skip: &[&str] = if skip_self { &[a_key] } else { &[] };
    crosscheck_between_multiline_geometry(
        a_store,
        b_store,
        a_key,
        a_window,
        targets,
        skip,
        evidence,
        COMPATIBILITY_GEOMETRY,
    )
    .map(|mut matches| {
        matches.truncate(k);
        matches
    })
}

/// `skip_keys` are B resources never voted (absent keys are ignored); see `Matcher::skipping`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn crosscheck_between_multiline_geometry(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    skip_keys: &[&str],
    evidence: bool,
    geometry: RegionGeometry,
) -> Result<Vec<CrosscheckMatch>> {
    ensure_compatible(a_store, b_store)?;
    let target_ids = if let Some(keys) = targets {
        let mut ids = HashSet::new();
        for key in keys {
            ids.insert(b_store.resource(key)?.id);
        }
        Some(ids)
    } else {
        None
    };
    let regions = region_prints(a_store, a_key, a_window, geometry)?;
    let matcher = Matcher::skipping(b_store, skip_keys);
    // Regions overlap (12 s every 8 s), and per-hash lookups over 64 shards dominate discover:
    // look every distinct hash up once, in one sorted walk per shard.
    let all_hashes: Vec<u64> = regions.iter().flatten().map(|print| print.hash).collect();
    let table = b_store.lookup_table(&all_hashes)?;
    let rows: Vec<_> = regions
        .par_iter()
        .map(|prints| matcher.match_prints_multiline_with(prints, usize::MAX, evidence, &table))
        .collect();
    // Grouping by reference must retain both chronological region order and the matcher's
    // score rank inside a region; the default b_start ordering would discard that rank.
    let mut by_key = BTreeMap::<String, Vec<(usize, usize, Segment)>>::new();
    for (region_index, rows) in rows.into_iter().enumerate() {
        for (line_rank, row) in rows?.into_iter().enumerate() {
            let resource = b_store.resource(&row.ref_key)?;
            if target_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&resource.id))
            {
                continue;
            }
            by_key.entry(row.ref_key.clone()).or_default().push((
                region_index,
                line_rank,
                row.into(),
            ));
        }
    }
    let mut matches: Vec<_> = by_key
        .into_iter()
        .map(|(ref_key, mut ranked)| {
            ranked.sort_by_key(|(region_index, line_rank, _)| (*region_index, *line_rank));
            let segments: Vec<_> = ranked.into_iter().map(|(_, _, segment)| segment).collect();
            CrosscheckMatch {
                ref_key,
                score_total: segments.iter().map(|segment| segment.score).sum(),
                segments,
            }
        })
        .collect();
    matches.sort_by(|a, b| {
        b.score_total
            .cmp(&a.score_total)
            .then_with(|| a.ref_key.cmp(&b.ref_key))
    });
    Ok(matches)
}

fn ensure_compatible(a_store: &Store, b_store: &Store) -> Result<()> {
    if a_store.config_id() != b_store.config_id() {
        return Err(Error::ConfigMismatch {
            expected: a_store.config_id().to_owned(),
            found: b_store.config_id().to_owned(),
        });
    }
    Ok(())
}

fn region_prints(
    store: &Store,
    key: &str,
    window: Option<(f64, f64)>,
    geometry: RegionGeometry,
) -> Result<Vec<Vec<crate::Fingerprint>>> {
    let resource = store.resource(key)?;
    let natural_stop = resource.t_max.saturating_add(1);
    if !geometry.window_seconds.is_finite()
        || !geometry.hop_seconds.is_finite()
        || geometry.window_seconds <= 0.0
        || geometry.hop_seconds <= 0.0
    {
        return Err(Error::Internal("invalid query region geometry".into()));
    }
    let (start, stop) = if let Some((start, stop)) = window {
        if !start.is_finite() || !stop.is_finite() || start < 0.0 || start >= stop {
            return Err(Error::BadRequest(
                "a_window must contain finite, non-negative increasing seconds".into(),
            ));
        }
        (
            seconds_to_bin(start).unwrap_or(0),
            seconds_to_bin(stop).unwrap_or(u32::MAX),
        )
    } else if geometry.anchor_at_zero {
        (0, natural_stop)
    } else {
        (resource.t_min, natural_stop)
    };
    let stop = stop.min(natural_stop);
    if start >= stop {
        return Err(Error::BadRequest(format!(
            "window contains no stored time range for {key:?}"
        )));
    }
    let region_bins = (geometry.window_seconds / TIME_BIN_SECONDS) as u32;
    let hop_bins = (geometry.hop_seconds / TIME_BIN_SECONDS) as u32;
    if region_bins == 0 || hop_bins == 0 {
        return Err(Error::Internal(
            "query region geometry is below one time bin".into(),
        ));
    }
    let mut regions = Vec::new();
    let mut cursor = start;
    while cursor < stop {
        let region_stop = cursor.saturating_add(region_bins).min(stop);
        let prints = store.forward(key, Some((cursor, region_stop)))?;
        if !prints.is_empty() {
            regions.push(prints);
        }
        cursor = cursor.saturating_add(hop_bins);
    }
    if regions.is_empty() {
        return Err(Error::BadRequest(format!(
            "window contains no fingerprints for {key:?}"
        )));
    }
    Ok(regions)
}

fn stable_segments(segments: &mut [Segment]) {
    segments.sort_by(|a, b| {
        a.a_start
            .total_cmp(&b.a_start)
            .then_with(|| a.b_start.total_cmp(&b.b_start))
            .then_with(|| b.score.cmp(&a.score))
    });
}

impl From<crate::MatchRow> for Segment {
    fn from(row: crate::MatchRow) -> Self {
        Self {
            a_start: row.q_start,
            a_stop: row.q_stop,
            b_start: row.ref_start,
            b_stop: row.ref_stop,
            score: row.score,
            time_factor: row.time_factor,
            pitch_factor: row.pitch_factor,
            sec_with_match: row.sec_with_match,
            evidence: row.evidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CROSSCHECK-BATCH: crosscheck reads its hits from one batched lookup table; every region
    /// must vote exactly the rows the per-hash lookup path gives, on real fixture prints.
    #[test]
    fn batched_crosscheck_regions_equal_per_hash_lookups() {
        let dump = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
        let root = tempfile::tempdir().unwrap();
        let store = Store::build(root.path(), crate::load_dump_dir(&dump).unwrap()).unwrap();
        let all = store.resources().len();
        let key = store.resources()[0].key.clone();
        let window = Some((0.0, 600.0));
        let regions = region_prints(&store, &key, window, COMPATIBILITY_GEOMETRY).unwrap();
        let matcher = Matcher::new(&store);
        let hashes: Vec<u64> = regions.iter().flatten().map(|print| print.hash).collect();
        let table = store.lookup_table(&hashes).unwrap();
        let mut rows = 0;
        for prints in &regions {
            let per_hash = matcher.match_prints(prints, all, true).unwrap();
            let batched = matcher
                .match_prints_with(prints, all, true, &table)
                .unwrap();
            assert_eq!(batched, per_hash);
            rows += per_hash.len();
        }
        assert!(
            rows >= regions.len(),
            "every region matches at least itself"
        );
        // And the public verb is that batched path, grouped.
        let found = crosscheck(&store, &key, window, None, all, false, false).unwrap();
        assert_eq!(found[0].ref_key, key);
    }
}
