use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod backup;
mod info;
mod list;
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
        /// Source file to backup
        source: PathBuf,
        /// Destination store directory
        destination: PathBuf,
        /// Average chunk size in bytes (default: 32768)
        #[arg(long, default_value_t = 32768)]
        chunk_size: usize,
        /// Similarity detection threshold (0.0-1.0, default: 0.7). Set to 0 to disable.
        #[arg(long, default_value_t = 0.7)]
        similarity_threshold: f64,
        /// Force re-backup even if file is unchanged
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// List backed up files
    List {
        /// Path to the store directory
        path: PathBuf,
    },
    /// Restore files from a backup
    Restore {
        /// Source store directory
        source: PathBuf,
        /// Destination path for restored data
        destination: PathBuf,
        /// Optional file name to restore (restores all if omitted)
        file: Option<String>,
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
            similarity_threshold,
            force,
        } => backup::run_backup(source, destination, *chunk_size, *similarity_threshold, *force),
        Commands::List { path } => list::run_list(path),
        Commands::Restore {
            source,
            destination,
            file,
        } => restore::run_restore(source, destination, file.as_deref()),
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
