#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use resident_core::{Store, load_dump_dir};

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
    }
    Ok(())
}
