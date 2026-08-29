#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use resident_core::{Store, crosscheck, extract_audio, load_dump_dir, span};

mod daemon;
mod extract_verify;
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
        a_key: String,
        #[arg(long)]
        b_key: String,
        #[arg(long, requires = "stop")]
        start: Option<f64>,
        #[arg(long, requires = "start")]
        stop: Option<f64>,
        #[arg(long)]
        evidence: bool,
    },
    /// Compare one stored resource against the store in a batched pass.
    Crosscheck {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        a_key: String,
        #[arg(long, default_value_t = 25)]
        k: usize,
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
            a_key,
            b_key,
            start,
            stop,
            evidence,
        } => {
            let store = Store::open(&store)?;
            let window = start.zip(stop);
            println!(
                "{}",
                serde_json::to_string_pretty(&span(&store, &a_key, window, &b_key, evidence)?)?
            );
        }
        Command::Crosscheck {
            store,
            a_key,
            k,
            evidence,
        } => {
            let store = Store::open(&store)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&crosscheck(
                    &store, &a_key, None, None, k, evidence
                )?)?
            );
        }
        Command::Retire { store, key } => {
            let (_, stats) = Store::retire(&store, &key)?;
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
    }
    Ok(())
}
