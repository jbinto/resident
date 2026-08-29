#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use resident_core::{Store, crosscheck, load_dump_dir, span};

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ping => println!("resident {}", env!("CARGO_PKG_VERSION")),
        Command::Ingest { store, dump_dir } => {
            let resources = load_dump_dir(&dump_dir)
                .with_context(|| format!("load dump {}", dump_dir.display()))?;
            let store = Store::build(&store, resources)
                .with_context(|| format!("build store {}", store.display()))?;
            println!("{}", serde_json::to_string_pretty(&store.stats())?);
        }
        Command::Stats { store } => {
            let store = Store::open(&store)?;
            println!("{}", serde_json::to_string_pretty(&store.stats())?);
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
    }
    Ok(())
}
