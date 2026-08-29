use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use resident_core::{Fingerprint, Matcher, Store, extract_audio, load_dump_dir, load_prints};
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    rows: Vec<GoldenRow>,
}

#[derive(Deserialize)]
struct GoldenRow {
    ref_path: String,
}

pub(crate) fn run(fixtures: &Path, store_path: Option<&Path>) -> anyhow::Result<()> {
    let temporary;
    let root = if let Some(path) = store_path {
        path
    } else {
        temporary = tempfile::tempdir().context("create temporary extraction store")?;
        temporary.path()
    };
    let store = if root.join("CURRENT").is_file() {
        Store::open(root)?
    } else {
        Store::build(root, load_dump_dir(&fixtures.join("store-dump"))?)?
    };
    let mut queries: Vec<PathBuf> = fs::read_dir(fixtures.join("queries"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    queries.retain(|path| path.is_dir());
    queries.sort();
    let matcher = Matcher::new(&store);
    let mut recall_sum = 0.0;
    let mut precision_sum = 0.0;
    let mut reference_hits = 0;
    let mut reference_total = 0;
    let mut anchor_recall_sum = 0.0;
    let mut anchor_precision_sum = 0.0;
    let mut elapsed = Duration::ZERO;
    for query in &queries {
        let name = query.file_name().unwrap().to_string_lossy();
        let expected = load_prints(&query.join("prints.tdb"))?;
        let started = Instant::now();
        let actual = extract_audio(&query.join("window.wav"))?;
        elapsed += started.elapsed();
        let recall = proximity_fraction(&expected, &actual.prints, 2);
        let precision = proximity_fraction(&actual.prints, &expected, 2);
        let expected_anchors: HashSet<_> =
            expected.iter().map(|print| (print.t, print.f)).collect();
        let actual_anchors: HashSet<_> = actual
            .prints
            .iter()
            .map(|print| (print.t, print.f))
            .collect();
        let anchor_recall = anchor_proximity_fraction(&expected_anchors, &actual_anchors, 2);
        let anchor_precision = anchor_proximity_fraction(&actual_anchors, &expected_anchors, 2);
        recall_sum += recall;
        precision_sum += precision;
        anchor_recall_sum += anchor_recall;
        anchor_precision_sum += anchor_precision;
        let golden: Golden = serde_json::from_slice(&fs::read(query.join("golden.json"))?)?;
        let expected_refs: HashSet<_> = golden.rows.into_iter().map(|row| row.ref_path).collect();
        let actual_rows = if actual.prints.is_empty() {
            Vec::new()
        } else {
            matcher.match_prints(&actual.prints, 25, false)?
        };
        let actual_refs: HashSet<_> = actual_rows.into_iter().map(|row| row.ref_key).collect();
        let found = expected_refs.intersection(&actual_refs).count();
        reference_hits += found;
        reference_total += expected_refs.len();
        println!(
            "{name:<20} jar={:<5} native={:<6} recall={:>6.1}% precision={:>6.1}% anchors={:>5.1}/{:>5.1}% refs={found}/{}",
            expected.len(),
            actual.prints.len(),
            recall * 100.0,
            precision * 100.0,
            anchor_recall * 100.0,
            anchor_precision * 100.0,
            expected_refs.len()
        );
    }
    let count = queries.len() as f64;
    println!(
        "\nextract: mean recall {:.1}%, mean precision {:.1}%, anchor recall {:.1}%, anchor precision {:.1}%, refs {reference_hits}/{reference_total}, total {:.3}s",
        recall_sum * 100.0 / count,
        precision_sum * 100.0 / count,
        anchor_recall_sum * 100.0 / count,
        anchor_precision_sum * 100.0 / count,
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn anchor_proximity_fraction(
    source: &HashSet<(u32, u16)>,
    candidates: &HashSet<(u32, u16)>,
    tolerance: u32,
) -> f64 {
    if source.is_empty() {
        return 1.0;
    }
    let matched = source
        .iter()
        .filter(|&&(time, frequency)| {
            let low = time.saturating_sub(tolerance);
            let high = time.saturating_add(tolerance);
            (low..=high).any(|candidate_time| candidates.contains(&(candidate_time, frequency)))
        })
        .count();
    matched as f64 / source.len() as f64
}

fn proximity_fraction(source: &[Fingerprint], candidates: &[Fingerprint], tolerance: u32) -> f64 {
    if source.is_empty() {
        return 1.0;
    }
    let mut by_hash = HashMap::<u64, Vec<u32>>::new();
    for print in candidates {
        by_hash.entry(print.hash).or_default().push(print.t);
    }
    for times in by_hash.values_mut() {
        times.sort_unstable();
    }
    let matched = source
        .iter()
        .filter(|print| {
            by_hash.get(&print.hash).is_some_and(|times| {
                let index = times.partition_point(|&time| time < print.t.saturating_sub(tolerance));
                times
                    .get(index)
                    .is_some_and(|&time| time <= print.t.saturating_add(tolerance))
            })
        })
        .count();
    matched as f64 / source.len() as f64
}
