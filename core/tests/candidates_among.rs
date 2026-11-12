//! TARGETED-CANDIDATES: `candidates_among` is a pair check that must answer, for every target,
//! exactly the row an unbounded `candidates` gives it. Real fixture prints, with time-shifted
//! slices so several references share material and land in different shards; once in one store
//! and once in a segmented store with a tombstone.

use std::collections::HashSet;
use std::path::Path;

use resident_core::config::seconds_to_bin;
use resident_core::{
    DumpResource, Fingerprint, Store, candidates, candidates_among, load_dump_dir, probe_window,
};

fn slice(source: &DumpResource, key: &str, from: f64, to: f64, shift: u32) -> DumpResource {
    let (from, to) = (seconds_to_bin(from).unwrap(), seconds_to_bin(to).unwrap());
    let mut slice = source.clone();
    slice.meta.key = key.to_owned();
    slice.prints = source
        .prints
        .iter()
        .filter(|print| print.t >= from && print.t < to)
        .map(|print| Fingerprint::new(print.hash, print.t - from + shift, print.f))
        .collect();
    slice.meta.declared_prints = slice.prints.len() as u64;
    slice.meta.duration = 200.0;
    slice
}

fn resources() -> (Vec<DumpResource>, Vec<DumpResource>) {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let base = load_dump_dir(&dump).expect("fixture dump");
    let slices = vec![
        slice(&base[0], "slice-a", 590.0, 700.0, 1000),
        slice(&base[0], "slice-b", 620.0, 730.0, 3000),
        slice(&base[0], "slice-c", 600.0, 640.0, 500),
        slice(&base[0], "slice-d", 650.0, 690.0, 0),
        slice(&base[1], "slice-e", 100.0, 160.0, 0),
        slice(&base[1], "slice-f", 120.0, 220.0, 700),
    ];
    (base, slices)
}

/// Probes: windows of the two sliced mixes and of an unrelated one, and whole slices.
fn probes(store: &Store, base: &[DumpResource]) -> Vec<(Vec<Fingerprint>, Option<f64>)> {
    let mut out = Vec::new();
    for (source, window) in [
        (&base[0], (580.0, 700.0)),
        (&base[0], (630.0, 660.0)),
        (&base[1], (90.0, 230.0)),
        (&base[2], (0.0, 120.0)),
    ] {
        out.push(probe_window(source.prints.clone(), Some(window), None).unwrap());
    }
    for key in ["slice-a", "slice-e"] {
        out.push((store.forward(key, None).unwrap(), Some(200.0)));
    }
    out
}

/// Every target set worth asking: all matched keys, each alone, the matched keys plus keys the
/// probe does not match, and only unmatched keys.
fn check_store(store: &Store, base: &[DumpResource]) {
    let keys: Vec<String> = store.resources().iter().map(|r| r.key.clone()).collect();
    let mut compared = 0;
    for (prints, duration) in probes(store, base) {
        let full = candidates(store, &prints, duration, usize::MAX).unwrap();
        assert!(full.len() >= 2, "a probe must match several references");
        let matched: Vec<String> = full.iter().map(|c| c.ref_key.clone()).collect();
        let unmatched: Vec<String> = keys
            .iter()
            .filter(|key| !matched.contains(key))
            .take(3)
            .cloned()
            .collect();
        let mut sets = vec![matched.clone(), unmatched.clone()];
        sets.extend(matched.iter().map(|key| vec![key.clone()]));
        sets.push(matched.iter().chain(&unmatched).rev().cloned().collect());
        for targets in sets {
            let wanted: HashSet<&String> = targets.iter().collect();
            let expected: Vec<_> = full
                .iter()
                .filter(|c| wanted.contains(&c.ref_key))
                .cloned()
                .collect();
            let among = candidates_among(store, &prints, duration, &targets, usize::MAX).unwrap();
            assert_eq!(among, expected, "targets {targets:?}");
            compared += expected.len();
        }
        // k still caps the targeted answer, best first.
        let top = candidates_among(store, &prints, duration, &matched, 1).unwrap();
        assert_eq!(top, full[..1].to_vec());
    }
    assert!(compared > 30, "only {compared} rows compared");
}

/// The cut-down table holds exactly the targets' hits in `lookup_table`'s order, and keeps each
/// hash's store-wide presence even when only other shards hold it (the voter's hit order depends
/// on it). The presence check must be exercised, not vacuous.
fn check_table(store: &Store, base: &[DumpResource]) {
    let (prints, _) = probe_window(base[0].prints.clone(), Some((580.0, 700.0)), None).unwrap();
    let hashes: Vec<u64> = prints.iter().map(|print| print.hash).collect();
    let full = store.lookup_table(&hashes).unwrap();
    let ids: Vec<u32> = ["slice-a", "slice-c"]
        .iter()
        .map(|key| store.resource(key).unwrap().id)
        .collect();
    let shards: HashSet<u32> = ids
        .iter()
        .map(|&id| store.resource_by_id(id).unwrap().shard)
        .collect();
    let among = store.lookup_table_among(&hashes, &ids).unwrap();
    let mut present_elsewhere_only = 0;
    for &hash in &hashes {
        let all = full.get(hash).unwrap();
        let (hits, present) = among.entry(hash).unwrap();
        let expected: Vec<_> = all
            .iter()
            .filter(|hit| ids.contains(&hit.resource_id))
            .copied()
            .collect();
        assert_eq!(hits, expected.as_slice(), "hits for {hash}");
        assert_eq!(present, !all.is_empty(), "presence of {hash}");
        if !all.is_empty()
            && all
                .iter()
                .all(|hit| !shards.contains(&store.resource_by_id(hit.resource_id).unwrap().shard))
        {
            present_elsewhere_only += 1;
        }
    }
    assert!(
        present_elsewhere_only > 0,
        "no hash was present only outside the targets' shards"
    );
}

#[test]
fn targeted_rows_equal_the_unbounded_rows() {
    let (base, slices) = resources();
    let root = tempfile::tempdir().unwrap();
    let mut all = base.clone();
    all.extend(slices);
    let store = Store::build(root.path(), all).unwrap();
    check_store(&store, &base);
    check_table(&store, &base);
}

#[test]
fn targeted_rows_equal_the_unbounded_rows_across_segments() {
    let (base, slices) = resources();
    let root = tempfile::tempdir().unwrap();
    Store::build(root.path(), base.clone()).unwrap();
    Store::append(root.path(), slices[..3].to_vec(), false, true).unwrap();
    let mut changed = slices[3..].to_vec();
    // A replaced key leaves a tombstone behind in the base segment.
    let mut replaced = base[3].clone();
    replaced.prints = replaced.prints.iter().copied().step_by(2).collect();
    replaced.meta.declared_prints = replaced.prints.len() as u64;
    changed.push(replaced);
    let (store, stats) = Store::append(root.path(), changed, true, true).unwrap();
    assert!(stats.segments >= 2, "{} segments", stats.segments);
    check_store(&store, &base);
    check_table(&store, &base);
}
