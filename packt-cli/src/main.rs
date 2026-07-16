use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod backup;
mod info;
mod restore;
mod verify;

#[derive(Parser)]
#[command(name = "packt")]
#[command(about = "Content-defined chunking with exact dedup for binary data")]
#[command(version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a deduplicated backup
    Backup {
        /// Source file or directory to backup
        source: PathBuf,
        /// Destination store directory
        destination: PathBuf,
        /// Average chunk size in bytes (default: 32768)
        #[arg(long, default_value_t = 32768)]
        chunk_size: usize,
    },
    /// Restore files from a backup
    Restore {
        /// Source store directory
        source: PathBuf,
        /// Destination path for restored data
        destination: PathBuf,
    },
    /// Show information about a backup store
    Info {
        /// Path to the store directory
        path: PathBuf,
    },
    /// Verify backup integrity
    Verify {
        /// Path to the store directory
        path: PathBuf,
    },
    /// Run performance benchmarks
    Benchmark {
        /// Directory with corpus data for benchmarking
        corpus: PathBuf,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Backup {
            source,
            destination,
            chunk_size,
        } => backup::run_backup(source, destination, *chunk_size),
        Commands::Restore { source, destination } => restore::run_restore(source, destination),
        Commands::Info { path } => info::run_info(path),
        Commands::Verify { path } => verify::run_verify(path),
        Commands::Benchmark { corpus } => {
            println!("Benchmark not yet implemented for corpus: {}", corpus.display());
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
