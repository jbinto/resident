//! DAEMON-POOL: a caller that pipes many heavy requests at once must get every answer, and the
//! same answers it gets asking one at a time. Before the fix every stdin line became a task in one
//! `rayon::scope`; a Rayon worker blocked inside one request's parallel loop stole the next
//! request, which stole the next, and the stack grew with the backlog until the process aborted
//! ("has overflowed its stack"). Small thread stacks (`RUST_MIN_STACK`) make that show on the
//! fixture store in a fraction of a second.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn fixture_store(root: &Path) {
    let dump = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
    let status = Command::new(env!("CARGO_BIN_EXE_resident"))
        .args(["ingest", "--store"])
        .arg(root)
        .arg("--dump-dir")
        .arg(&dump)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "fixture ingest");
}

fn keys(root: &Path) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_resident"))
        .args(["stats", "--store"])
        .arg(root)
        .output()
        .unwrap();
    let stats: Value = serde_json::from_slice(&output.stdout).unwrap();
    stats["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["key"].as_str().unwrap().to_owned())
        .collect()
}

fn requests(keys: &[String], copies: usize) -> Vec<Value> {
    (0..copies)
        .flat_map(|copy| {
            keys.iter().map(move |key| {
                json!({"id": format!("{copy}:{key}"), "verb": "crosscheck", "a_key": key, "a_window": [0.0, 60.0],
                       "targets": "all", "k": 10, "skip_self": true})
            })
        })
        .collect()
}

/// Run the daemon with `stack`-byte thread stacks and `workers` request threads. `flood` writes
/// every request before reading any answer; otherwise each answer is read before the next request
/// goes out.
fn run(
    root: &Path,
    requests: &[Value],
    flood: bool,
    stack: usize,
    workers: usize,
) -> (BTreeMap<String, Value>, String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_resident"))
        .args(["daemon", "--workers", &workers.to_string(), "--store"])
        .arg(root)
        .env("RUST_MIN_STACK", stack.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut answers = BTreeMap::new();
    let mut read_one = |answers: &mut BTreeMap<String, Value>| {
        let mut line = String::new();
        if stdout.read_line(&mut line).unwrap() == 0 {
            return false;
        }
        let answer: Value = serde_json::from_str(&line).unwrap();
        answers.insert(answer["id"].as_str().unwrap().to_owned(), answer);
        true
    };
    if flood {
        let lines: String = requests
            .iter()
            .map(|request| format!("{request}\n"))
            .collect();
        let writer = std::thread::spawn(move || {
            // A dead daemon closes the pipe; that failure shows up as missing answers below.
            let _ = stdin.write_all(lines.as_bytes());
        });
        while read_one(&mut answers) {}
        writer.join().unwrap();
    } else {
        for request in requests {
            if writeln!(stdin, "{request}").is_err() || !read_one(&mut answers) {
                break;
            }
        }
        drop(stdin);
    }
    let output = child.wait_with_output().unwrap();
    (
        answers,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn a_flood_of_heavy_requests_gets_every_answer_and_the_same_answers() {
    let store = tempfile::tempdir().unwrap();
    fixture_store(store.path());
    let keys = keys(store.path());
    assert!(keys.len() >= 10, "fixture store has its resources");

    // The smallest stack on which every request, asked alone, succeeds (debug frames are bigger
    // than release ones, so this is measured, not assumed). Nesting requests overflows it.
    let one_copy = requests(&keys, 1);
    let (stack, sequential) = [64, 128, 256, 512, 1024]
        .into_iter()
        .map(|kib| kib * 1024)
        .find_map(|stack| {
            let (answers, _, ok) = run(store.path(), &one_copy, false, stack, 1);
            (ok && answers.len() == keys.len()).then_some((stack, answers))
        })
        .expect("one request at a time fits some small stack");

    // One request in flight: nothing else is queued in Rayon for a blocked worker to steal, so the
    // flood needs about the stack one request needs. Twice that leaves room for how Rayon happens
    // to split one request on a loaded machine; per-line Rayon tasks nested dozens of whole
    // requests and still overflow it. (The default N workers bound the nesting at N requests.)
    let stack = 2 * stack;
    let flood = requests(&keys, 20);
    let (flooded, stderr, ok) = run(store.path(), &flood, true, stack, 1);
    assert!(
        ok && flooded.len() == flood.len(),
        "flood on {stack}-byte stacks: {} of {} answered, clean exit {ok}; stderr: {}",
        flooded.len(),
        flood.len(),
        stderr.lines().last().unwrap_or("")
    );
    let mut nonempty = 0;
    for (id, answer) in &flooded {
        assert_eq!(answer["ok"], true, "{id}: {answer}");
        let key = id.split_once(':').unwrap().1;
        let alone = &sequential[&format!("0:{key}")];
        assert_eq!(answer["matches"], alone["matches"], "{id} differs");
        nonempty += usize::from(!answer["matches"].as_array().unwrap().is_empty());
    }
    assert!(nonempty > 0, "some probe has a real match to compare");
}
