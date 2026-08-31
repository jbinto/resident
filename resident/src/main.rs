#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use resident_core::{
    Store, crosscheck_between, crosscheck_between_multiline, discover_passages_between,
    extract_audio, load_dump_dir, passages_between, span_between, span_between_multiline,
};

mod ab_compare;
mod daemon;
mod durations;
mod extract_verify;
mod refingerprint;
mod verify;

#[derive(Debug, Parser)]
#[command(
    name = "resident",
    version,
    about = "Resident audio fingerprint engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate that the binary is runnable.
    Ping,
    /// Build a new store generation from a Panako dump directory.
    Ingest {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        dump_dir: PathBuf,
        #[arg(long)]
        replace: bool,
    },
    /// Print store statistics as JSON.
    Stats {
        #[arg(long)]
        store: PathBuf,
    },
    /// Reproduce all matcher oracle fixtures and print a parity report.
    Verify {
        fixtures: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Compare one stored resource window against another resource.
    Span {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        b_store: Option<PathBuf>,
        #[arg(long)]
        a_key: String,
        #[arg(long)]
        b_key: String,
        #[arg(long, requires = "stop")]
        start: Option<f64>,
        #[arg(long, requires = "start")]
        stop: Option<f64>,
        #[arg(long)]
        evidence: bool,
        #[arg(long)]
        multi_line: bool,
    },
    /// Compare one stored resource against the store in a batched pass.
    Crosscheck {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        b_store: Option<PathBuf>,
        #[arg(long)]
        a_key: String,
        #[arg(long, default_value_t = 25)]
        k: usize,
        #[arg(long)]
        evidence: bool,
        #[arg(long)]
        multi_line: bool,
    },
    /// Form geometry-compatible passages between two stored resources.
    Passages {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        b_store: Option<PathBuf>,
        #[arg(long)]
        a_key: String,
        #[arg(long)]
        b_key: String,
        #[arg(long, requires = "stop")]
        start: Option<f64>,
        #[arg(long, requires = "start")]
        stop: Option<f64>,
        /// Include the raw evidence-bearing region lines behind each passage.
        #[arg(long)]
        evidence: bool,
    },
    /// Exhaustively discover passage matches for one stored resource.
    Discover {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        b_store: Option<PathBuf>,
        #[arg(long)]
        a_key: String,
        /// Restrict discovery to these target keys. Repeat for multiple resources.
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Omit a caller-known sibling encoding of the same logical audio revision.
        #[arg(long = "exclude-key")]
        exclude_keys: Vec<String>,
        #[arg(long, requires = "stop")]
        start: Option<f64>,
        #[arg(long, requires = "start")]
        stop: Option<f64>,
        /// Include the raw evidence-bearing region lines behind each passage.
        #[arg(long)]
        evidence: bool,
    },
    /// Retire a resource by publishing a new generation without its postings.
    Retire {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        key: String,
    },
    /// Publish prints-only endpoint identities without rewriting fingerprint shards.
    RehashIdentities {
        #[arg(long)]
        store: PathBuf,
    },
    /// Publish authoritative duration metadata without changing fingerprint identity.
    SetDurations {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        durations: PathBuf,
        #[arg(long)]
        expected_generation: String,
    },
    /// Serve the v0 JSON-lines protocol over stdin/stdout.
    Daemon {
        #[arg(long)]
        store: PathBuf,
    },
    /// Extract native fingerprints from an audio file.
    Extract { audio_path: PathBuf },
    /// Measure native extraction against fixture prints and match answers.
    ValidateExtract {
        fixtures: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Prove bounded-memory extraction is identical to whole-file decoding.
    ValidateStream { fixtures: PathBuf },
    /// Prove opt-in secondary lines on a real cross-match fixture.
    ValidateMultiline {
        fixtures: PathBuf,
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Extract a resumable Panako-grammar dump set from a JSONL audio manifest.
    Refingerprint {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
        #[arg(long, default_value_t = 1)]
        jobs: usize,
        #[arg(long, default_value_t = 25)]
        progress_every: usize,
    },
    /// Compare identical match questions against two stores.
    AbCompare {
        #[arg(long)]
        a_store: PathBuf,
        #[arg(long)]
        b_store: PathBuf,
        #[arg(
            long,
            required_unless_present = "questions",
            conflicts_with = "questions"
        )]
        probes_dir: Option<PathBuf>,
        #[arg(
            long,
            required_unless_present = "probes_dir",
            conflicts_with = "probes_dir"
        )]
        questions: Option<PathBuf>,
        #[arg(long, default_value_t = 25)]
        k: usize,
        #[arg(long, default_value_t = 0)]
        max_score_delta: usize,
        #[arg(long, default_value_t = 20)]
        largest: usize,
        #[arg(long)]
        evidence: Option<String>,
    },
    /// Exercise store agreement against the real fixtures and one retirement.
    ValidateAb { fixtures: PathBuf },
    /// Prove cross-store span on a split fixture corpus.
    ValidateCrossStore { fixtures: PathBuf },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ping => println!("resident {}", env!("CARGO_PKG_VERSION")),
        Command::Ingest {
            store,
            dump_dir,
            replace,
        } => {
            let resources = load_dump_dir(&dump_dir)
                .with_context(|| format!("load dump {}", dump_dir.display()))?;
            let (_, stats) = Store::ingest(&store, resources, replace)
                .with_context(|| format!("ingest store {}", store.display()))?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Command::Stats { store } => {
            let store = Store::open(&store)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "store": store.stats(),
                    "resources": store.resources(),
                }))?
            );
        }
        Command::Verify { fixtures, store } => verify::run(&fixtures, store.as_deref())?,
        Command::Span {
            store,
            b_store,
            a_key,
            b_key,
            start,
            stop,
            evidence,
            multi_line,
        } => {
            let store = Store::open(&store)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(&store);
            let window = start.zip(stop);
            let segments = if multi_line {
                span_between_multiline(&store, target_store, &a_key, window, &b_key, evidence)?
            } else {
                span_between(&store, target_store, &a_key, window, &b_key, evidence)?
            };
            println!("{}", serde_json::to_string_pretty(&segments)?);
        }
        Command::Crosscheck {
            store,
            b_store,
            a_key,
            k,
            evidence,
            multi_line,
        } => {
            let store = Store::open(&store)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(&store);
            let matches = if multi_line {
                crosscheck_between_multiline(&store, target_store, &a_key, None, None, k, evidence)?
            } else {
                crosscheck_between(&store, target_store, &a_key, None, None, k, evidence)?
            };
            println!("{}", serde_json::to_string_pretty(&matches)?);
        }
        Command::Passages {
            store,
            b_store,
            a_key,
            b_key,
            start,
            stop,
            evidence,
        } => {
            let store = Store::open(&store)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(&store);
            let answer = passages_between(
                &store,
                target_store,
                &a_key,
                start.zip(stop),
                &b_key,
                evidence,
            )?;
            println!("{}", serde_json::to_string_pretty(&answer)?);
        }
        Command::Discover {
            store,
            b_store,
            a_key,
            targets,
            exclude_keys,
            start,
            stop,
            evidence,
        } => {
            let store = Store::open(&store)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(&store);
            let answer = discover_passages_between(
                &store,
                target_store,
                &a_key,
                start.zip(stop),
                (!targets.is_empty()).then_some(targets.as_slice()),
                (!exclude_keys.is_empty()).then_some(exclude_keys.as_slice()),
                evidence,
            )?;
            println!("{}", serde_json::to_string_pretty(&answer)?);
        }
        Command::Retire { store, key } => {
            let (_, stats) = Store::retire(&store, &key)?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Command::RehashIdentities { store } => {
            let (_, stats) = Store::rehash_identities(&store)?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Command::SetDurations {
            store,
            durations,
            expected_generation,
        } => {
            let updates = durations::load(&durations)?;
            let (_, stats) = Store::set_durations(&store, &expected_generation, updates)?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Command::Daemon { store } => daemon::run(store)?,
        Command::Extract { audio_path } => {
            let extraction = extract_audio(&audio_path)?;
            let prints: Vec<_> = extraction
                .prints
                .iter()
                .map(|print| [print.hash, u64::from(print.t), u64::from(print.f)])
                .collect();
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "prints": prints,
                    "duration": extraction.duration,
                }))?
            );
        }
        Command::ValidateExtract { fixtures, store } => {
            extract_verify::run(&fixtures, store.as_deref())?
        }
        Command::ValidateStream { fixtures } => extract_verify::run_stream(&fixtures)?,
        Command::ValidateMultiline { fixtures, store } => {
            verify::run_multiline(&fixtures, store.as_deref())?
        }
        Command::Refingerprint {
            manifest,
            output_dir,
            jobs,
            progress_every,
        } => refingerprint::run(&manifest, &output_dir, jobs, progress_every)?,
        Command::AbCompare {
            a_store,
            b_store,
            probes_dir,
            questions,
            k,
            max_score_delta,
            largest,
            evidence,
        } => ab_compare::run(ab_compare::Options {
            a_store,
            b_store,
            probes_dir,
            questions,
            k,
            max_score_delta,
            largest,
            evidence,
        })?,
        Command::ValidateAb { fixtures } => ab_compare::validate(&fixtures)?,
        Command::ValidateCrossStore { fixtures } => verify::run_cross_store(&fixtures)?,
    }
    Ok(())
}
