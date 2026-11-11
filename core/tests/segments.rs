//! SEGMENTS: a store grown by appends and tombstones answers exactly as the store in-place
//! ingest and retire write, and compacting it writes that very store (same manifest, same shard
//! files). Real fixture prints, with time-shifted slices so references share material and hit
//! order across segments matters.

use std::fs::File;
use std::path::Path;

use resident_core::config::seconds_to_bin;
use resident_core::{
    DumpResource, Fingerprint, Store, candidates, crosscheck, crosscheck_multiline,
    discover_passages_between, load_dump_dir,
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

/// The base store's resources, two appended batches (the second replaces a base key) and the
/// keys retired at the end.
fn history() -> (
    Vec<DumpResource>,
    Vec<DumpResource>,
    Vec<DumpResource>,
    Vec<String>,
) {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let mut all = load_dump_dir(&dump).expect("fixture dump");
    let late = all.split_off(12);
    let base = all;
    let mut first = late[..2].to_vec();
    first.push(slice(&base[0], "slice-a", 590.0, 700.0, 1000));
    first.push(slice(&base[0], "slice-b", 620.0, 730.0, 3000));
    let mut second = late[2..].to_vec();
    first.push(slice(&second[0], "slice-c", 100.0, 160.0, 0));
    // A changed copy of a base key: every other print of it.
    let mut changed = base[3].clone();
    changed.prints = changed.prints.iter().copied().step_by(2).collect();
    changed.meta.declared_prints = changed.prints.len() as u64;
    second.push(changed);
    second.push(slice(&base[0], "slice-d", 600.0, 640.0, 500));
    let retired = vec![base[5].meta.key.clone(), "slice-b".to_owned()];
    (base, first, second, retired)
}

fn answers(store: &Store) -> Vec<String> {
    let mut keys: Vec<String> = store.resources().iter().map(|r| r.key.clone()).collect();
    keys.sort();
    let all = keys.len();
    let mut out = Vec::new();
    for key in &keys {
        let multi = crosscheck_multiline(store, key, None, None, all, true, false).unwrap();
        let single = crosscheck(store, key, Some((0.0, 300.0)), None, all, false, false).unwrap();
        out.push(format!(
            "{key} {} {}",
            serde_json::to_string(&multi).unwrap(),
            serde_json::to_string(&single).unwrap()
        ));
    }
    for key in keys.iter().filter(|key| key.starts_with("slice")) {
        let found = discover_passages_between(store, store, key, None, None, None, false).unwrap();
        out.push(serde_json::to_string(&found.matches).unwrap());
        let prints = store.forward(key, None).unwrap();
        out.push(serde_json::to_string(&candidates(store, &prints, None, all).unwrap()).unwrap());
    }
    // Raw lookups over every hash of two resources, plus the global population test.
    let mut hashes: Vec<u64> = ["slice-a", keys[0].as_str()]
        .iter()
        .filter_map(|key| store.forward(key, None).ok())
        .flatten()
        .map(|print| print.hash)
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    let table = store.lookup_table(&hashes).unwrap();
    for hash in hashes {
        let hits = store.lookup(hash).unwrap();
        assert_eq!(
            table.get(hash).unwrap(),
            hits.as_slice(),
            "table vs lookup {hash}"
        );
        out.push(format!(
            "{hash} {} {hits:?}",
            store.has_match(hash).unwrap()
        ));
    }
    out
}

fn manifest(root: &Path) -> String {
    let current = std::fs::read_to_string(root.join("CURRENT")).unwrap();
    std::fs::read_to_string(root.join("generations").join(current.trim())).unwrap()
}

#[test]
fn appends_and_tombstones_answer_as_the_rewritten_store_and_compact_to_it() {
    let (base, first, second, retired) = history();

    let rewritten = tempfile::tempdir().unwrap();
    Store::build(rewritten.path(), base.clone()).unwrap();
    Store::ingest(rewritten.path(), first.clone(), false).unwrap();
    Store::ingest(rewritten.path(), second.clone(), true).unwrap();
    for key in &retired {
        Store::retire(rewritten.path(), key).unwrap();
    }
    let rewritten_store = Store::open(rewritten.path()).unwrap();

    let segmented = tempfile::tempdir().unwrap();
    Store::build(segmented.path(), base).unwrap();
    let (_, stats) = Store::append(segmented.path(), first, false, true).unwrap();
    assert_eq!(stats.segments, 2);
    let (_, stats) = Store::append(segmented.path(), second, true, true).unwrap();
    assert_eq!((stats.segments, stats.resources_replaced), (3, 1));
    let (segmented_store, stats) = Store::tombstone(segmented.path(), &retired).unwrap();
    assert_eq!((stats.segments, stats.merges), (3, 0));
    assert_eq!(
        segmented_store.stats().postings,
        rewritten_store.stats().postings
    );
    // In-place ingest/retire refuse a segmented store rather than mixing the two layouts.
    assert!(Store::retire(segmented.path(), "slice-a").is_err());

    let expected = answers(&rewritten_store);
    assert_eq!(answers(&segmented_store), expected);

    let (compacted, stats) = Store::compact(segmented.path(), true).unwrap();
    assert_eq!((stats.segments_before, stats.segments), (3, 1));
    assert!(stats.postings_dropped > 0);
    assert_eq!(answers(&compacted), expected);
    assert_eq!(manifest(segmented.path()), manifest(rewritten.path()));
    assert_eq!(
        compacted.stats().generation,
        rewritten_store.stats().generation
    );
}

#[test]
fn tiered_merges_bound_the_segment_count_and_keep_answers() {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let resources = load_dump_dir(&dump).expect("fixture dump");
    let rewritten = tempfile::tempdir().unwrap();
    let segmented = tempfile::tempdir().unwrap();
    Store::build(rewritten.path(), resources[..4].to_vec()).unwrap();
    Store::build(segmented.path(), resources[..4].to_vec()).unwrap();
    let mut most = 0;
    let mut merges = 0;
    for (index, resource) in resources[4..].iter().enumerate() {
        let night = vec![
            resource.clone(),
            slice(&resources[0], &format!("night-{index}"), 590.0, 650.0, 0),
        ];
        Store::ingest(rewritten.path(), night.clone(), false).unwrap();
        let (_, stats) = Store::append(segmented.path(), night, false, true).unwrap();
        most = most.max(stats.segments);
        merges += stats.merges;
    }
    // Every small segment is in tier 0, so four of them merge as soon as they exist.
    assert!(merges >= 3, "{merges} merges");
    assert!(most <= 4, "at most {most} segments");
    let rewritten = Store::open(rewritten.path()).unwrap();
    let segmented = Store::open(segmented.path()).unwrap();
    assert_eq!(answers(&segmented), answers(&rewritten));
}

#[test]
fn a_second_writer_is_refused() {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let resources = load_dump_dir(&dump).expect("fixture dump");
    let root = tempfile::tempdir().unwrap();
    Store::build(root.path(), resources[..3].to_vec()).unwrap();
    let holder = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.path().join("LOCK"))
        .unwrap();
    holder.lock().unwrap();
    let error = Store::append(root.path(), resources[3..4].to_vec(), false, true)
        .err()
        .expect("a held lock refuses a second writer");
    assert!(error.to_string().contains("another writer"), "{error}");
    assert!(Store::compact(root.path(), true).is_err());
    assert!(Store::ingest(root.path(), resources[3..4].to_vec(), false).is_err());
    drop(holder);
    Store::append(root.path(), resources[3..4].to_vec(), false, true).unwrap();
}
