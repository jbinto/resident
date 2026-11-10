//! STORE-MERGE and BOUNDED-BUILD: merging stores, or building shard by shard from files, must
//! produce exactly the store one in-memory build of the same resources makes:
//! the same manifest and the same shard bytes, on real fixture prints.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use resident_core::{DumpResource, Error, Store, load_dump_dir};

fn fixture_resources() -> Vec<DumpResource> {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    load_dump_dir(&dump).expect("fixture dump")
}

/// Every file under the published generation: CURRENT, its manifest, and the shards it names.
fn published_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let current = fs::read_to_string(root.join("CURRENT")).unwrap();
    let manifest = root.join("generations").join(current.trim());
    let parsed: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let mut files = BTreeMap::new();
    files.insert("CURRENT".to_owned(), current.into_bytes());
    files.insert("manifest".to_owned(), fs::read(&manifest).unwrap());
    for shard in parsed["shards"].as_array().unwrap() {
        let name = shard["file"].as_str().unwrap();
        files.insert(
            name.to_owned(),
            fs::read(root.join("shards").join(name)).unwrap(),
        );
    }
    files
}

#[test]
fn merge_equals_one_build_of_the_union() {
    let resources = fixture_resources();
    assert!(resources.len() >= 6);
    let temporary = tempfile::tempdir().unwrap();
    let union = Store::build(&temporary.path().join("union"), resources.clone()).unwrap();

    // Three inputs; the last one grown by an append, so its ids are not in key order.
    let (mut parts, mut tail) = (vec![Vec::new(), Vec::new()], Vec::new());
    for (index, resource) in resources.into_iter().enumerate() {
        match index % 3 {
            2 => tail.push(resource),
            part => parts[part].push(resource),
        }
    }
    let mut inputs = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        inputs.push(Store::build(&temporary.path().join(format!("part{index}")), part).unwrap());
    }
    let late = tail.split_off(tail.len() / 2);
    let grown = temporary.path().join("grown");
    Store::build(&grown, late).unwrap();
    let (grown_store, _) = Store::ingest(&grown, tail, false).unwrap();
    inputs.push(grown_store);

    let merged_root = temporary.path().join("merged");
    let merged = Store::merge(&merged_root, &inputs).unwrap();
    assert_eq!(merged.stats().generation, union.stats().generation);
    assert_eq!(merged.stats().postings, union.stats().postings);
    let expected = published_bytes(&temporary.path().join("union"));
    let found = published_bytes(&merged_root);
    assert_eq!(expected.len(), 66, "CURRENT, manifest and 64 shards");
    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &expected {
        assert!(found[name] == *bytes, "{name} differs");
    }
}

#[test]
fn merge_refuses_shared_keys_and_an_existing_output() {
    let resources = fixture_resources();
    let temporary = tempfile::tempdir().unwrap();
    let a = Store::build(&temporary.path().join("a"), resources[..3].to_vec()).unwrap();
    let b = Store::build(&temporary.path().join("b"), resources[2..5].to_vec()).unwrap();
    let error = Store::merge(&temporary.path().join("ab"), &[a, b])
        .err()
        .expect("merge must refuse");
    assert!(matches!(error, Error::BadRequest(ref message) if message.contains("duplicate")));

    let c = Store::build(&temporary.path().join("c"), resources[5..6].to_vec()).unwrap();
    let error = Store::merge(&temporary.path().join("a"), &[c])
        .err()
        .expect("merge must refuse");
    assert!(matches!(error, Error::BadRequest(ref message) if message.contains("already")));
}

#[test]
fn bounded_build_from_files_equals_the_in_memory_build() {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let temporary = tempfile::tempdir().unwrap();
    let in_memory = temporary.path().join("in-memory");
    Store::build(&in_memory, fixture_resources()).unwrap();
    let bounded = temporary.path().join("bounded");
    Store::build_from_dump_dir(&bounded, &dump).unwrap();
    let expected = published_bytes(&in_memory);
    assert_eq!(expected.len(), 66, "CURRENT, manifest and 64 shards");
    let found = published_bytes(&bounded);
    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &expected {
        assert!(found[name] == *bytes, "{name} differs");
    }
}
