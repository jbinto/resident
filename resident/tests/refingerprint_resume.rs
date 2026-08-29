use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn killed_batch_resumes_to_byte_identical_dump_set() {
    let temporary = tempfile::tempdir().expect("temporary test directory");
    let audio = temporary.path().join("short.wav");
    write_short_wav(&audio);
    let manifest = temporary.path().join("manifest.jsonl");
    let manifest_text: String = (0..40)
        .map(|index| {
            format!(
                "{{\"key\":\"resource-{index:02}\",\"audio_path\":{}}}\n",
                serde_json::to_string(&audio).expect("serialize audio path")
            )
        })
        .collect();
    fs::write(&manifest, manifest_text).expect("write manifest");
    let resumed = temporary.path().join("resumed");
    let clean = temporary.path().join("clean");

    let mut child = command(&manifest, &resumed)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start interrupted batch");
    let deadline = Instant::now() + Duration::from_secs(10);
    let completed_before_kill = loop {
        let count = metadata_count(&resumed);
        if count > 0 {
            child.kill().expect("kill batch");
            child.wait().expect("reap killed batch");
            break count;
        }
        assert!(
            child.try_wait().expect("poll batch").is_none(),
            "batch completed before it could be interrupted"
        );
        assert!(
            Instant::now() < deadline,
            "batch did not publish a resource"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(completed_before_kill < 40);

    let resumed_output = command(&manifest, &resumed)
        .output()
        .expect("resume interrupted batch");
    assert!(resumed_output.status.success());
    let summary: serde_json::Value =
        serde_json::from_slice(&resumed_output.stdout).expect("parse resumed summary");
    assert_eq!(summary["total"], 40);
    assert_eq!(summary["failed"], 0);
    assert!(summary["skipped"].as_u64().expect("numeric skipped") > 0);

    let clean_output = command(&manifest, &clean)
        .output()
        .expect("run clean batch");
    assert!(clean_output.status.success());
    assert_eq!(directory_bytes(&resumed), directory_bytes(&clean));
}

#[test]
fn decode_failure_is_recorded_without_aborting_other_resources() {
    let temporary = tempfile::tempdir().expect("temporary test directory");
    let audio = temporary.path().join("short.wav");
    write_short_wav(&audio);
    let missing = temporary.path().join("missing.wav");
    let manifest = temporary.path().join("manifest.jsonl");
    let text = format!(
        "{{\"key\":\"good\",\"audio_path\":{}}}\n{{\"key\":\"bad\",\"audio_path\":{}}}\n",
        serde_json::to_string(&audio).expect("serialize good path"),
        serde_json::to_string(&missing).expect("serialize missing path")
    );
    fs::write(&manifest, text).expect("write manifest");
    let output_dir = temporary.path().join("output");
    let output = command(&manifest, &output_dir)
        .output()
        .expect("run failure batch");
    assert!(output.status.success());
    let summary: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse failure summary");
    assert_eq!(summary["done"], 1);
    assert_eq!(summary["failed"], 1);
    let failures = fs::read_to_string(output_dir.join("failures.jsonl"))
        .expect("read deterministic failures file");
    let failure: serde_json::Value =
        serde_json::from_str(failures.trim()).expect("parse failure record");
    assert_eq!(failure["id"], 2);
    assert_eq!(failure["key"], "bad");
    assert!(output_dir.join("1_meta_data.txt").is_file());
    assert!(!output_dir.join("2_meta_data.txt").exists());
}

fn command(manifest: &Path, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resident"));
    command
        .arg("refingerprint")
        .arg("--manifest")
        .arg(manifest)
        .arg("--output-dir")
        .arg(output)
        .arg("--jobs")
        .arg("1")
        .arg("--progress-every")
        .arg("1000");
    command
}

fn metadata_count(path: &Path) -> usize {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.ends_with("_meta_data.txt"))
                })
                .count()
        })
        .unwrap_or(0)
}

fn directory_bytes(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(path)
        .expect("read output directory")
        .map(|entry| entry.expect("read output entry").path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = PathBuf::from(path.file_name().expect("output file name"));
            let bytes = fs::read(path).expect("read output file");
            (name, bytes)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn write_short_wav(path: &Path) {
    let sample_count = 1_600_u32;
    let data_bytes = sample_count * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&16_000_u32.to_le_bytes());
    bytes.extend_from_slice(&32_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    bytes.resize(bytes.len() + data_bytes as usize, 0);
    fs::write(path, bytes).expect("write short wav");
}
