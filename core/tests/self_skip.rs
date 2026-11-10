//! SELF-SKIP: never voting the probe's own key must leave every other reference's answer
//! byte-identical to today's answer with the self row filtered out afterwards. Skipped postings
//! still count toward the global matching-hash population (Panako's iteration order), which is
//! what this test guards on real fixture prints.

use std::path::Path;

use resident_core::config::seconds_to_bin;
use resident_core::{
    DumpResource, Fingerprint, ResourceMeta, Store, crosscheck, crosscheck_multiline,
    discover_passages_between, load_dump_dir,
};

/// The fixture corpus plus two overlapping, time-shifted slices of its first resource, so the
/// probe windows below have real sibling matches besides themselves.
fn fixture_store(root: &Path) -> Store {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let mut resources = load_dump_dir(&dump).expect("fixture dump");
    let source = resources[0].clone();
    for (name, from, to, shift) in [
        ("slice-a", 590.0, 700.0, 1000),
        ("slice-b", 620.0, 730.0, 3000),
    ] {
        let (from, to) = (seconds_to_bin(from).unwrap(), seconds_to_bin(to).unwrap());
        let mut slice = source.clone();
        slice.meta.key = name.to_owned();
        slice.prints = source
            .prints
            .iter()
            .filter(|print| print.t >= from && print.t < to)
            .map(|print| Fingerprint::new(print.hash, print.t - from + shift, print.f))
            .collect();
        slice.meta.declared_prints = slice.prints.len() as u64;
        slice.meta.duration = 200.0;
        resources.push(slice);
    }
    Store::build(root, resources).expect("fixture store")
}

fn first_key(store: &Store) -> String {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let first = load_dump_dir(&dump)
        .expect("fixture dump")
        .remove(0)
        .meta
        .key;
    assert!(store.resource(&first).is_ok());
    first
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("serializable")
}

#[test]
fn skipping_self_equals_filtering_self_afterwards() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture_store(root.path());
    let keys: Vec<String> = store.resources().iter().map(|r| r.key.clone()).collect();
    let all = keys.len();
    let mut self_rows = 0;
    for key in &keys {
        let window = (!key.starts_with("slice-")).then_some((600.0, 720.0));
        for multi_line in [false, true] {
            let ask = |skip_self| {
                if multi_line {
                    crosscheck_multiline(&store, key, window, None, all, false, skip_self)
                } else {
                    crosscheck(&store, key, window, None, all, false, skip_self)
                }
                .expect("crosscheck")
            };
            let mut filtered = ask(false);
            let before = filtered.len();
            filtered.retain(|found| &found.ref_key != key);
            self_rows += before - filtered.len();
            let skipped = ask(true);
            assert!(skipped.iter().all(|found| &found.ref_key != key));
            assert_eq!(
                json(&skipped),
                json(&filtered),
                "{key} multi_line={multi_line}"
            );
        }
    }
    // The comparison is only meaningful if the self diagonal was really there to skip.
    assert_eq!(self_rows, 2 * all);
}

#[test]
fn discover_matches_when_exclusions_are_never_voted() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture_store(root.path());
    let source = first_key(&store);
    let probes = [
        (source.as_str(), Some((600.0, 690.0))),
        ("slice-a", None),
        ("slice-b", None),
    ];
    for (key, window) in probes {
        let full = discover_passages_between(&store, &store, key, window, None, None, false)
            .expect("discover");
        assert!(full.matches.iter().all(|found| found.ref_key != key));
        assert!(full.matches.len() >= 2, "{key}: both siblings expected");
        // Excluding each found reference removes exactly that row and changes nothing else,
        // although its postings still shape the global hash population.
        for excluded in full.matches.iter().map(|found| found.ref_key.clone()) {
            let fewer = discover_passages_between(
                &store,
                &store,
                key,
                window,
                None,
                Some(std::slice::from_ref(&excluded)),
                false,
            )
            .expect("discover");
            let mut expected = full.matches.clone();
            expected.retain(|found| found.ref_key != excluded);
            assert_eq!(
                json(&fewer.matches),
                json(&expected),
                "{key} without {excluded}"
            );
        }
    }
}

fn synthetic(key: &str, prints: Vec<Fingerprint>) -> DumpResource {
    DumpResource {
        meta: ResourceMeta {
            source_id: "1".into(),
            key: key.into(),
            duration: 20.0,
            declared_prints: prints.len() as u64,
        },
        prints,
        prints_path: Default::default(),
    }
}

/// A tie that only the global matching-hash population can break. Probe `s` has 12 times with two
/// prints each (hashes a_i, b_i) plus 40 hashes only `s` holds. Reference `r` holds a_i at offset 5
/// and b_i at offset 21: two equally voted lines in the same Java bucket, so the first hit decides.
/// Which of a_i/b_i comes first follows Java's bucket order at the population's capacity: 32 slots
/// for the 24 shared hashes alone, 128 once `s`'s own 40 count too. a_i = 1024i+37 and b_i =
/// 1024i+10 order one way at 32 slots and the other way at 128. So `r`'s row changes unless the
/// skipped `s` still counts toward the population.
#[test]
fn skipped_postings_still_count_toward_the_hash_population() {
    let times: Vec<u32> = (0..12).map(|i| i * 100).collect();
    let pair = |i: usize| (1024 * i as u64 + 37, 1024 * i as u64 + 10);
    let mut probe = Vec::new();
    let mut reference = Vec::new();
    for (i, &t) in times.iter().enumerate() {
        let (a, b) = pair(i);
        probe.extend([Fingerprint::new(a, t, 100), Fingerprint::new(b, t, 100)]);
        reference.extend([
            Fingerprint::new(a, t + 5, 100),
            Fingerprint::new(b, t + 21, 100),
        ]);
    }
    for j in 0..40 {
        probe.push(Fingerprint::new(
            500_000 + 16 * j,
            times[(j as usize) % 12],
            100,
        ));
    }
    let temporary = tempfile::tempdir().unwrap();
    let with_probe = Store::build(
        &temporary.path().join("with"),
        vec![
            synthetic("s", probe.clone()),
            synthetic("r", reference.clone()),
        ],
    )
    .unwrap();
    // The same probe and reference without the 40 hashes only the probe holds.
    let without_probe = Store::build(
        &temporary.path().join("without"),
        vec![
            synthetic("s", probe[..24].to_vec()),
            synthetic("r", reference),
        ],
    )
    .unwrap();
    let row = |store: &Store, skip_self| {
        let found = crosscheck(
            store,
            "s",
            None,
            Some(&["r".to_owned()]),
            1,
            false,
            skip_self,
        )
        .expect("crosscheck");
        assert_eq!(found.len(), 1, "r must match");
        json(&found[0])
    };
    let unskipped = row(&with_probe, false);
    assert_eq!(row(&with_probe, true), unskipped);
    // The case is sensitive: with a smaller population the tie breaks the other way.
    assert_ne!(row(&without_probe, true), unskipped);
}
