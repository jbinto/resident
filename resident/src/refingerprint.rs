use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, bail};
use rayon::prelude::*;
use resident_core::{Error, Fingerprint, extract_audio_streaming, load_metadata, load_prints};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MANIFEST_MARKER: &str = "REFINGERPRINT_MANIFEST.sha256";
const FAILURES_FILE: &str = "failures.jsonl";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestLine {
    key: String,
    audio_path: PathBuf,
}

#[derive(Clone, Debug)]
struct WorkItem {
    id: u32,
    key: String,
    audio_path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct Failure {
    id: u32,
    key: String,
    audio_path: PathBuf,
    error: String,
}

enum Outcome {
    Done,
    Failed(Failure),
}

#[derive(Debug, Serialize)]
struct Summary {
    total: usize,
    done: usize,
    skipped: usize,
    failed: usize,
    failures_file: PathBuf,
}

pub(crate) fn run(
    manifest_path: &Path,
    output_dir: &Path,
    jobs: usize,
    progress_every: usize,
) -> anyhow::Result<()> {
    if jobs == 0 {
        bail!("--jobs must be greater than zero");
    }
    if progress_every == 0 {
        bail!("--progress-every must be greater than zero");
    }
    let manifest_bytes = fs::read(manifest_path)
        .with_context(|| format!("read manifest {}", manifest_path.display()))?;
    let items = parse_manifest(manifest_path, &manifest_bytes)?;
    verify_ffmpeg()?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create output directory {}", output_dir.display()))?;
    pin_manifest(output_dir, &manifest_bytes)?;

    let mut pending = Vec::new();
    let mut skipped = 0;
    let total = items.len();
    let scan_started = Instant::now();
    for (index, item) in items.into_iter().enumerate() {
        if completed(output_dir, &item)? {
            skipped += 1;
        } else {
            pending.push(item);
        }
        let checked = index + 1;
        if checked.is_multiple_of(progress_every) || checked == total {
            eprintln!(
                "refingerprint scan={checked}/{total} completed={skipped} elapsed={:.1}s",
                scan_started.elapsed().as_secs_f64()
            );
        }
    }

    let processed = AtomicUsize::new(skipped);
    let done = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    debug_assert_eq!(total, skipped + pending.len());
    let started = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .thread_name(|index| format!("refingerprint-{index}"))
        .build()
        .context("build refingerprint worker pool")?;
    let outcomes: Vec<anyhow::Result<Outcome>> = pool.install(|| {
        pending
            .par_iter()
            .map(|item| {
                let outcome = process_one(output_dir, item)?;
                match &outcome {
                    Outcome::Done => {
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                    Outcome::Failed(_) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let count = processed.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(progress_every) || count == total {
                    eprintln!(
                        "refingerprint progress={count}/{total} done={} skipped={skipped} failed={} elapsed={:.1}s",
                        done.load(Ordering::Relaxed),
                        failed.load(Ordering::Relaxed),
                        started.elapsed().as_secs_f64()
                    );
                }
                Ok(outcome)
            })
            .collect()
    });
    let mut failures = Vec::new();
    for outcome in outcomes {
        if let Outcome::Failed(failure) = outcome? {
            failures.push(failure);
        }
    }
    failures.sort_by_key(|failure| failure.id);
    let failures_path = output_dir.join(FAILURES_FILE);
    write_failures(&failures_path, &failures)?;
    sync_directory(output_dir)?;

    let summary = Summary {
        total,
        done: done.load(Ordering::Relaxed),
        skipped,
        failed: failures.len(),
        failures_file: failures_path,
    };
    println!("{}", serde_json::to_string(&summary)?);
    Ok(())
}

fn verify_ffmpeg() -> anyhow::Result<()> {
    let status = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("start ffmpeg; refingerprint requires it on PATH")?;
    if !status.success() {
        bail!("ffmpeg -version failed with {status}");
    }
    Ok(())
}

fn parse_manifest(path: &Path, bytes: &[u8]) -> anyhow::Result<Vec<WorkItem>> {
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("manifest {} is not UTF-8", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut keys = HashSet::new();
    let mut items = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            bail!(
                "{}:{line_number}: blank manifest lines are not allowed",
                path.display()
            );
        }
        let parsed: ManifestLine = serde_json::from_str(line)
            .with_context(|| format!("parse {}:{line_number}", path.display()))?;
        if parsed.key.is_empty() {
            bail!("{}:{line_number}: key must not be empty", path.display());
        }
        if parsed.audio_path.as_os_str().is_empty() {
            bail!(
                "{}:{line_number}: audio_path must not be empty",
                path.display()
            );
        }
        if !keys.insert(parsed.key.clone()) {
            bail!(
                "{}:{line_number}: duplicate key {:?}",
                path.display(),
                parsed.key
            );
        }
        let id = u32::try_from(line_number).context("manifest has more than u32::MAX lines")?;
        let audio_path = if parsed.audio_path.is_absolute() {
            parsed.audio_path
        } else {
            base.join(parsed.audio_path)
        };
        items.push(WorkItem {
            id,
            key: parsed.key,
            audio_path,
        });
    }
    if items.is_empty() {
        bail!("manifest {} contains no resources", path.display());
    }
    Ok(items)
}

fn pin_manifest(output_dir: &Path, manifest: &[u8]) -> anyhow::Result<()> {
    let digest = format!("{:x}\n", Sha256::digest(manifest));
    let marker = output_dir.join(MANIFEST_MARKER);
    if marker.exists() {
        let existing = fs::read_to_string(&marker)
            .with_context(|| format!("read manifest marker {}", marker.display()))?;
        if existing != digest {
            bail!(
                "output directory {} belongs to a different manifest",
                output_dir.display()
            );
        }
        return Ok(());
    }
    atomic_write(&marker, digest.as_bytes())?;
    sync_directory(output_dir)
}

fn completed(output_dir: &Path, item: &WorkItem) -> anyhow::Result<bool> {
    let metadata_path = output_dir.join(format!("{}_meta_data.txt", item.id));
    let prints_path = output_dir.join(format!("{}.tdb", item.id));
    if !metadata_path.exists() {
        return Ok(false);
    }
    if !prints_path.is_file() {
        bail!(
            "completed metadata {} has no matching print file",
            metadata_path.display()
        );
    }
    let metadata = load_metadata(&metadata_path)?;
    if metadata.source_id != item.id.to_string() || metadata.key != item.key {
        bail!(
            "completed output {} does not match manifest id/key",
            metadata_path.display()
        );
    }
    let prints = load_prints(&prints_path)?;
    if prints.len() as u64 != metadata.declared_prints {
        bail!(
            "completed output {} declares {} prints but contains {}",
            metadata_path.display(),
            metadata.declared_prints,
            prints.len()
        );
    }
    Ok(true)
}

fn process_one(output_dir: &Path, item: &WorkItem) -> anyhow::Result<Outcome> {
    let prints_path = output_dir.join(format!("{}.tdb", item.id));
    let prints_partial = output_dir.join(format!("{}.tdb.partial", item.id));
    let file = File::create(&prints_partial)
        .with_context(|| format!("create {}", prints_partial.display()))?;
    let mut writer = BufWriter::new(file);
    let mut count = 0_u64;
    let mut write_error = None;
    let extraction = extract_audio_streaming(&item.audio_path, |print| {
        if write_error.is_none() {
            if let Err(error) = write_print(&mut writer, item.id, print) {
                write_error = Some(error);
            } else {
                count += 1;
            }
        }
    });
    let duration = match extraction {
        Ok(duration) => duration,
        Err(Error::BadRequest(message)) => {
            drop(writer);
            let _ = fs::remove_file(&prints_partial);
            return Ok(Outcome::Failed(Failure {
                id: item.id,
                key: item.key.clone(),
                audio_path: item.audio_path.clone(),
                error: message,
            }));
        }
        Err(error) => return Err(error.into()),
    };
    if let Some(error) = write_error {
        return Err(error).with_context(|| format!("write {}", prints_partial.display()));
    }
    writer
        .flush()
        .with_context(|| format!("flush {}", prints_partial.display()))?;
    let file = writer
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)
        .with_context(|| format!("finish {}", prints_partial.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", prints_partial.display()))?;
    fs::rename(&prints_partial, &prints_path).with_context(|| {
        format!(
            "publish {} as {}",
            prints_partial.display(),
            prints_path.display()
        )
    })?;

    let metadata_path = output_dir.join(format!("{}_meta_data.txt", item.id));
    let metadata = format!("{}\n{}\n{}\n{}\n", item.id, duration, count, item.key);
    atomic_write(&metadata_path, metadata.as_bytes())?;
    sync_directory(output_dir)?;
    Ok(Outcome::Done)
}

fn write_print(writer: &mut impl Write, id: u32, print: Fingerprint) -> std::io::Result<()> {
    writeln!(writer, "{} {} {} {} ", print.hash, id, print.t, print.f)
}

fn write_failures(path: &Path, failures: &[Failure]) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    for failure in failures {
        serde_json::to_writer(&mut bytes, failure)?;
        bytes.push(b'\n');
    }
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let partial = path.with_extension(format!(
        "{}.partial",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
    ));
    let mut file = File::create(&partial)
        .with_context(|| format!("create temporary file {}", partial.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write temporary file {}", partial.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temporary file {}", partial.display()))?;
    fs::rename(&partial, path)
        .with_context(|| format!("publish {} as {}", partial.display(), path.display()))?;
    Ok(())
}

fn sync_directory(path: &Path) -> anyhow::Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync directory {}", path.display()))
}
