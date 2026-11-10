//! STDERR-DRAIN: an ffmpeg that writes more than a pipe buffer to stderr must not deadlock
//! extraction. A shim `ffmpeg` first on PATH floods stderr (1 MiB, 16x a 64 KiB pipe) before
//! writing PCM. Before the fix, ffmpeg blocked on its full stderr pipe while resident blocked
//! reading stdout, forever; here that shows up as a timeout.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn shim(dir: &Path, exit_code: i32) {
    let script = format!(
        "#!/bin/sh\n\
         head -c 1048576 /dev/zero | tr '\\000' 'x' >&2\n\
         printf 'last words\\n' >&2\n\
         head -c 320000 /dev/zero\n\
         exit {exit_code}\n"
    );
    let path = dir.join("ffmpeg");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `resident extract` with the shim first on PATH, killed if it outlives `limit`.
fn extract_with_shim(dir: &Path, limit: Duration) -> Output {
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_resident"))
        .args(["extract", "ignored-by-the-shim.mp3"])
        .env("PATH", std::env::join_paths(paths).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > limit {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("extraction deadlocked on a flooded ffmpeg stderr");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn a_flooded_stderr_neither_hangs_nor_floods_the_error() {
    let ok = tempfile::tempdir().unwrap();
    shim(ok.path(), 0);
    let output = extract_with_shim(ok.path(), Duration::from_secs(60));
    assert!(
        output.status.success(),
        "a successful decode stays successful"
    );
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["duration"].as_f64(), Some(10.0), "10 s of PCM");

    let failing = tempfile::tempdir().unwrap();
    shim(failing.path(), 1);
    let output = extract_with_shim(failing.path(), Duration::from_secs(60));
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("last words"),
        "the error quotes stderr's end"
    );
    assert!(
        error.len() < 8192,
        "but not the whole flood ({} bytes)",
        error.len()
    );
}
