//! SEGMENTS crash safety: `resident append` and `resident compact --all` stopped dead (abort) at
//! every durable step of a publish always leave a store that opens to a whole generation, the old
//! one or the new one, and the next writer finishes the job over whatever the dead one left.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use resident_core::{Store, crosscheck_multiline};

fn resident(args: &[&str], paths: &[&Path]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resident"));
    command.args(args);
    command.args(paths);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command
}

fn fixture_dump() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump")
}

/// The store's shard for a key (store.rs `shard_for_key`); checked against the built store below.
fn shard_of(key: &str) -> u32 {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(key.as_bytes());
    u32::from_le_bytes(hash[..4].try_into().unwrap()) % 64
}

/// Copies of `count` fixture dump entries under new keys (`<prefix>-<n>-<salt>`), each salted so
/// it lands in shard 0 or 1: every batch then shares those shards, so a merge rewrites them.
fn renamed_dump(dir: &Path, prefix: &str, count: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let mut metas: Vec<_> = std::fs::read_dir(fixture_dump())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with("_meta_data.txt"))
        .collect();
    metas.sort();
    for (index, meta) in metas.into_iter().take(count).enumerate() {
        let name = meta.file_name().unwrap().to_string_lossy().into_owned();
        let id = name.trim_end_matches("_meta_data.txt");
        let text = std::fs::read_to_string(&meta).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines[3] = (0..)
            .map(|salt| format!("{prefix}-{index}-{salt}"))
            .find(|key| shard_of(key) == index as u32 % 2)
            .unwrap();
        // The first 20,000 prints only (about a minute and a half): enough for real matches,
        // small enough that every writer run in this test is quick.
        let compressed = std::fs::read(fixture_dump().join(format!("{id}.tdb.zst"))).unwrap();
        let text = String::from_utf8(zstd::decode_all(compressed.as_slice()).unwrap()).unwrap();
        let prints: Vec<&str> = text.lines().take(20_000).collect();
        lines[2] = prints.len().to_string();
        std::fs::write(dir.join(&name), lines.join("\n") + "\n").unwrap();
        std::fs::write(dir.join(format!("{id}.tdb")), prints.join("\n") + "\n").unwrap();
    }
}

fn copy_dir(from: &Path, to: &Path) {
    let status = Command::new("cp")
        .arg("-R")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Every durable step a publish passes (core store.rs `crash_point`), and passes deep into the
/// 64 shard writes of a segment.
const CRASH_POINTS: [&str; 8] = [
    "shard-temp:1",
    "shard-temp:2",
    "shard:1",
    "shard:2",
    "manifest-temp:1",
    "current-temp:1",
    "current:1",
    "current:2",
];

/// Run `command` stopped dead at `point`; true if it died there (the point was reached).
fn crash_at(mut command: Command, point: &str) -> bool {
    !command
        .env("RESIDENT_TEST_CRASH_AT", point)
        .status()
        .unwrap()
        .success()
}

fn answers(store: &Store) -> Vec<String> {
    let mut keys: Vec<String> = store.resources().iter().map(|r| r.key.clone()).collect();
    keys.sort();
    keys.iter()
        .take(2)
        .map(|key| {
            let found =
                crosscheck_multiline(store, key, Some((0.0, 60.0)), None, 5, false, false).unwrap();
            serde_json::to_string(&found).unwrap()
        })
        .collect()
}

#[test]
fn killed_writers_leave_a_whole_generation() {
    let work = tempfile::tempdir().unwrap();
    let pristine = work.path().join("pristine");
    let base = work.path().join("base");
    let more = work.path().join("more");
    let later = work.path().join("later");
    renamed_dump(&base, "base", 4);
    renamed_dump(&more, "more", 3);
    renamed_dump(&later, "later", 2);
    assert!(
        resident(&["ingest", "--store"], &[&pristine])
            .arg("--dump-dir")
            .arg(&base)
            .status()
            .unwrap()
            .success()
    );
    let opened = Store::open(&pristine).unwrap();
    assert!(
        opened
            .resources()
            .iter()
            .all(|r| r.shard == shard_of(&r.key))
    );
    let before = opened.stats().resources;
    drop(opened);

    // Mid-append: the store opens with or without the whole batch, never part of it.
    let mut interrupted = 0;
    for (round, point) in CRASH_POINTS.into_iter().enumerate() {
        let store = work.path().join(format!("append-{round}"));
        copy_dir(&pristine, &store);
        let mut command = resident(&["append", "--store"], &[&store]);
        command.arg("--dump-dir").arg(&more);
        interrupted += usize::from(crash_at(command, point));
        let opened = Store::open(&store).expect("a killed append leaves an openable store");
        let resources = opened.stats().resources;
        assert!(
            resources == before || resources == before + 3,
            "{point}: {resources} resources"
        );
        answers(&opened);
        drop(opened);
        // The next writer finishes the job over whatever the dead one left.
        let mut again = resident(&["append", "--store"], &[&store]);
        assert!(
            again
                .arg("--dump-dir")
                .arg(&more)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(Store::open(&store).unwrap().stats().resources, before + 3);
    }
    // An append publishes once (no merge is due), so only "current:2" is never reached.
    assert_eq!(interrupted, CRASH_POINTS.len() - 1, "appends stopped");

    // Mid-compact: a three-segment store compacts to the same answers or stays as it was.
    let segmented = work.path().join("segmented");
    copy_dir(&pristine, &segmented);
    for dump in [&more, &later] {
        let mut append = resident(&["append", "--no-merge", "--store"], &[&segmented]);
        assert!(
            append
                .arg("--dump-dir")
                .arg(dump)
                .status()
                .unwrap()
                .success()
        );
    }
    let expected = answers(&Store::open(&segmented).unwrap());
    let mut interrupted = 0;
    for (round, point) in CRASH_POINTS.into_iter().enumerate() {
        let store = work.path().join(format!("compact-{round}"));
        copy_dir(&segmented, &store);
        let command = resident(&["compact", "--all", "--store"], &[&store]);
        interrupted += usize::from(crash_at(command, point));
        let opened = Store::open(&store).expect("a killed compact leaves an openable store");
        assert_eq!(opened.stats().resources, before + 5);
        assert_eq!(answers(&opened), expected, "{point}");
        drop(opened);
        assert!(
            resident(&["compact", "--all", "--store"], &[&store])
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(answers(&Store::open(&store).unwrap()), expected);
    }
    assert_eq!(interrupted, CRASH_POINTS.len() - 1, "compactions stopped");
}
