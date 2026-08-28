use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use resident_core::{MatchRow, Matcher, Store, load_dump_dir, load_prints};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Golden {
    query: String,
    rows: Vec<GoldenRow>,
}

#[derive(Debug, Deserialize)]
struct GoldenRow {
    q_start: f64,
    q_stop: f64,
    ref_path: String,
    ref_start: f64,
    ref_stop: f64,
    score: f64,
    time_factor: f64,
    pitch_factor: f64,
}

pub(crate) fn run(fixtures: &Path, store_path: Option<&Path>) -> anyhow::Result<()> {
    let temporary;
    let root = if let Some(path) = store_path {
        path
    } else {
        temporary = tempfile::tempdir().context("create temporary fixture store")?;
        temporary.path()
    };
    let store = if root.join("CURRENT").is_file() {
        Store::open(root)?
    } else {
        Store::build(root, load_dump_dir(&fixtures.join("store-dump"))?)?
    };
    let mut query_dirs: Vec<PathBuf> = fs::read_dir(fixtures.join("queries"))
        .context("read fixture queries")?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    query_dirs.retain(|path| path.is_dir());
    query_dirs.sort();
    if query_dirs.is_empty() {
        bail!("{} has no query fixtures", fixtures.display());
    }

    let matcher = Matcher::new(&store);
    let mut passed = 0;
    let mut total_time = Duration::ZERO;
    let mut failures = Vec::new();
    for query_dir in &query_dirs {
        let golden_path = query_dir.join("golden.json");
        let golden: Golden = serde_json::from_slice(
            &fs::read(&golden_path).with_context(|| format!("read {}", golden_path.display()))?,
        )
        .with_context(|| format!("parse {}", golden_path.display()))?;
        let prints = load_prints(&query_dir.join("prints.tdb"))?;
        let started = Instant::now();
        let actual = matcher.match_prints(&prints, 25, false)?;
        let elapsed = started.elapsed();
        total_time += elapsed;
        let differences = compare(&golden.rows, &actual);
        if differences.is_empty() {
            passed += 1;
            println!(
                "PASS {:<20} rows={:<2} score_max={:<4} {:>8.3} ms",
                golden.query,
                actual.len(),
                actual.first().map_or(0, |row| row.score),
                elapsed.as_secs_f64() * 1000.0
            );
        } else {
            println!(
                "FAIL {:<20} rows={} {:>8.3} ms",
                golden.query,
                actual.len(),
                elapsed.as_secs_f64() * 1000.0
            );
            for difference in &differences {
                println!("  {difference}");
            }
            failures.push(golden.query);
        }
    }
    println!(
        "\nverify: {passed}/{} passed; match total {:.3} ms, mean {:.3} ms",
        query_dirs.len(),
        total_time.as_secs_f64() * 1000.0,
        total_time.as_secs_f64() * 1000.0 / query_dirs.len() as f64
    );
    if !failures.is_empty() {
        bail!("fixture parity failed: {}", failures.join(", "));
    }
    Ok(())
}

fn compare(expected: &[GoldenRow], actual: &[MatchRow]) -> Vec<String> {
    let expected: BTreeMap<_, _> = expected
        .iter()
        .map(|row| (row.ref_path.as_str(), row))
        .collect();
    let actual: BTreeMap<_, _> = actual
        .iter()
        .map(|row| (row.ref_key.as_str(), row))
        .collect();
    let mut differences = Vec::new();
    for missing in expected.keys().filter(|key| !actual.contains_key(**key)) {
        differences.push(format!("missing reference {missing}"));
    }
    for extra in actual.keys().filter(|key| !expected.contains_key(**key)) {
        differences.push(format!("extra reference {extra}"));
    }
    for (key, expected) in expected {
        let Some(actual) = actual.get(key) else {
            continue;
        };
        if expected.score != actual.score as f64 {
            differences.push(format!(
                "{key}: score expected {}, got {}",
                expected.score, actual.score
            ));
        }
        compare_float(
            &mut differences,
            key,
            "q_start",
            expected.q_start,
            actual.q_start,
            0.02,
        );
        compare_float(
            &mut differences,
            key,
            "q_stop",
            expected.q_stop,
            actual.q_stop,
            0.02,
        );
        compare_float(
            &mut differences,
            key,
            "ref_start",
            expected.ref_start,
            actual.ref_start,
            0.02,
        );
        compare_float(
            &mut differences,
            key,
            "ref_stop",
            expected.ref_stop,
            actual.ref_stop,
            0.02,
        );
        compare_float(
            &mut differences,
            key,
            "time_factor",
            expected.time_factor,
            actual.time_factor,
            0.001,
        );
        compare_float(
            &mut differences,
            key,
            "pitch_factor",
            expected.pitch_factor,
            actual.pitch_factor,
            0.001,
        );
    }
    differences
}

fn compare_float(
    differences: &mut Vec<String>,
    key: &str,
    field: &str,
    expected: f64,
    actual: f64,
    tolerance: f64,
) {
    if (expected - actual).abs() > tolerance {
        differences.push(format!(
            "{key}: {field} expected {expected:.6}, got {actual:.6}"
        ));
    }
}
