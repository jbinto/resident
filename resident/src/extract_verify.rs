use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Context;
use resident_core::{
    Fingerprint, Matcher, Store, extract_audio, extract_audio_whole, load_dump_dir, load_prints,
};
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

pub(crate) fn run_stream(fixtures: &Path) -> anyhow::Result<()> {
    let queries = query_directories(fixtures)?;
    for query in &queries {
        let name = query.file_name().unwrap().to_string_lossy();
        compare_streams(&query.join("window.wav"))?;
        println!("{name:<20} stream=exact");
    }

    // The delivered windows are exactly one canonical extraction core. Stitch three real
    // windows so this gate also crosses core boundaries and exercises final-window flushing.
    let temporary = tempfile::tempdir().context("create stitched stream fixture")?;
    let stitched = temporary.path().join("three-windows.wav");
    let inputs: Vec<_> = queries
        .iter()
        .take(3)
        .map(|query| query.join("window.wav"))
        .collect();
    anyhow::ensure!(
        inputs.len() == 3,
        "validate-stream needs at least three fixture windows"
    );
    let status = Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-v")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(&inputs[0])
        .arg("-i")
        .arg(&inputs[1])
        .arg("-i")
        .arg(&inputs[2])
        .arg("-filter_complex")
        .arg("[0:a][1:a][2:a]concat=n=3:v=0:a=1[out]")
        .arg("-map")
        .arg("[out]")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&stitched)
        .status()
        .context("start ffmpeg for stitched stream fixture")?;
    anyhow::ensure!(status.success(), "ffmpeg could not stitch stream fixture");
    let prints = compare_streams(&stitched)?;
    println!("stitched_3x12s       stream=exact prints={prints}");
    println!(
        "\nstream: {}/{} fixture windows exact; boundary/flush exact",
        queries.len(),
        queries.len()
    );
    Ok(())
}

fn query_directories(fixtures: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut queries: Vec<PathBuf> = fs::read_dir(fixtures.join("queries"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    queries.retain(|path| path.is_dir());
    queries.sort();
    Ok(queries)
}

fn compare_streams(path: &Path) -> anyhow::Result<usize> {
    let whole = extract_audio_whole(path)?;
    let streamed = extract_audio(path)?;
    anyhow::ensure!(
        whole.duration.to_bits() == streamed.duration.to_bits(),
        "stream duration differs for {}: whole={} stream={}",
        path.display(),
        whole.duration,
        streamed.duration
    );
    anyhow::ensure!(
        whole.prints == streamed.prints,
        "stream fingerprints differ for {}: whole={} stream={}",
        path.display(),
        whole.prints.len(),
        streamed.prints.len()
    );
    Ok(streamed.prints.len())
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
