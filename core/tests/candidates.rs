//! CANDIDATES on real fixture prints: an excerpt is covered on the probe side only, a whole
//! stored recording on both sides, and coverage comes from matched spans, not from scores.

use std::path::Path;

use resident_core::config::{TIME_BIN_SECONDS, seconds_to_bin};
use resident_core::{Fingerprint, Relation, Store, candidates, load_dump_dir, probe_window};

#[test]
fn coverage_both_ways_names_the_relation() {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let mut resources = load_dump_dir(&dump).expect("fixture dump");
    let mix = resources[0].clone();
    // A 90-second recording cut out of the first mix, stored as its own resource.
    let (from, to) = (
        seconds_to_bin(600.0).unwrap(),
        seconds_to_bin(690.0).unwrap(),
    );
    let mut cut = mix.clone();
    cut.meta.key = "cut".into();
    cut.meta.duration = 90.0;
    cut.prints = mix
        .prints
        .iter()
        .filter(|print| print.t >= from && print.t < to)
        .map(|print| Fingerprint::new(print.hash, print.t - from, print.f))
        .collect();
    cut.meta.declared_prints = cut.prints.len() as u64;
    resources.push(cut.clone());
    let root = tempfile::tempdir().unwrap();
    let store = Store::build(root.path(), resources).unwrap();

    // The whole cut as a probe: it is all of "cut" and a small part of the mix.
    let found = candidates(&store, &cut.prints, Some(90.0), 5).unwrap();
    let by_key = |key: &str| found.iter().find(|found| found.ref_key == key).unwrap();
    let whole = by_key("cut");
    assert_eq!(whole.relation, Relation::Full);
    assert!(whole.query_coverage > 0.8 && whole.reference_coverage > 0.8);
    assert!(whole.offset_seconds.abs() < 0.1);
    let source = by_key(&mix.meta.key);
    assert_eq!(source.relation, Relation::QueryWithinReference);
    assert!(source.query_coverage > 0.8);
    assert!(source.reference_coverage < 0.1);
    assert!(
        (source.offset_seconds - 600.0).abs() < 1.0,
        "{}",
        source.offset_seconds
    );
    assert!((source.time_factor - 1.0).abs() < 0.01);

    // A 20-second window of the mix: an excerpt of both the mix and the cut.
    let (window, duration) = probe_window(mix.prints.clone(), Some((620.0, 640.0)), None).unwrap();
    assert_eq!(duration, Some(20.0));
    let found = candidates(&store, &window, duration, 5).unwrap();
    let cut_row = found.iter().find(|found| found.ref_key == "cut").unwrap();
    assert_eq!(cut_row.relation, Relation::QueryWithinReference);
    assert!(cut_row.reference_coverage < 0.4);
    assert!(
        (cut_row.offset_seconds + 600.0).abs() < 1.0,
        "{}",
        cut_row.offset_seconds
    );
}

/// A reference that shares only one second in four with the probe (as an instrumental shares the
/// beat under a vocal) spans the whole probe, so span coverage calls it the same recording. Its
/// hits never form dense runs, so supported coverage, and the relation, say shared material.
#[test]
fn a_sparse_line_is_partial_not_full() {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let mut resources = load_dump_dir(&dump).expect("fixture dump");
    let mix = resources[0].clone();
    let (from, to) = (
        seconds_to_bin(600.0).unwrap(),
        seconds_to_bin(690.0).unwrap(),
    );
    let one_second = (1.0 / TIME_BIN_SECONDS) as u32;
    let cut: Vec<_> = mix
        .prints
        .iter()
        .filter(|print| print.t >= from && print.t < to)
        .map(|print| Fingerprint::new(print.hash, print.t - from, print.f))
        .collect();
    let mut thin = mix.clone();
    thin.meta.key = "thin".into();
    thin.meta.duration = 90.0;
    thin.prints = cut
        .iter()
        .copied()
        .filter(|print| (print.t / one_second).is_multiple_of(4))
        .collect();
    thin.meta.declared_prints = thin.prints.len() as u64;
    resources.push(thin);
    let root = tempfile::tempdir().unwrap();
    let store = Store::build(root.path(), resources).unwrap();

    let found = candidates(&store, &cut, Some(90.0), 5).unwrap();
    let thin = found.iter().find(|found| found.ref_key == "thin").unwrap();
    assert!(
        thin.query_coverage > 0.8 && thin.reference_coverage > 0.8,
        "the sparse line spans the probe: {thin:?}"
    );
    assert!(
        thin.query_supported_coverage < 0.35 && thin.reference_supported_coverage < 0.35,
        "but its hits are not dense: {thin:?}"
    );
    assert_eq!(thin.relation, Relation::Partial);
    // The dense source keeps its relation.
    let source = found
        .iter()
        .find(|found| found.ref_key == mix.meta.key)
        .unwrap();
    assert_eq!(source.relation, Relation::QueryWithinReference);
    assert!(source.query_supported_coverage > 0.8, "{source:?}");
}
