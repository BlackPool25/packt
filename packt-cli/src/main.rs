use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod backup;
mod info;
mod list;
mod migrate;
mod restore;
mod verify;

/// Global CLI options shared across all commands.
#[derive(Debug, Clone, Copy)]
pub struct GlobalOpts {
    pub json: bool,
    pub verbose: bool,
    pub quiet: bool,
}

#[derive(Parser)]
#[command(name = "packt")]
#[command(about = "Content-defined chunking with exact dedup for binary data")]
#[command(version, long_about = None)]
struct Cli {
    /// JSON output mode
    #[arg(global = true, long, default_value_t = false)]
    json: bool,
    /// Verbose output
    #[arg(global = true, short, long, default_value_t = false, conflicts_with = "quiet")]
    verbose: bool,
    /// Suppress non-error output
    #[arg(global = true, short, long, default_value_t = false)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a deduplicated backup
    Backup {
        /// Source file to backup
        source: PathBuf,
        /// Store URI: /local/path, s3://bucket/key, gcs://bucket/key
        destination: String,
        /// Chunk size preset or raw bytes (default: 8k). Presets: 8k, 32k, 64k.
        #[arg(long, default_value_t = String::from("8k"))]
        chunk_size: String,
        /// Similarity detection threshold (0.0-1.0, default: 0.7). Set to 0 to disable.
        #[arg(long, default_value_t = 0.7)]
        similarity_threshold: f64,
        /// Force re-backup even if file is unchanged
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// List backed up files
    List {
        /// Store URI: /local/path, s3://bucket/key, gcs://bucket/key
        store: String,
    },
    /// Restore files from a backup
    Restore {
        /// Store URI: /local/path, s3://bucket/key, gcs://bucket/key
        source: String,
        /// Destination path for restored data
        destination: PathBuf,
        /// Optional file name to restore (restores all if omitted)
        file: Option<String>,
    },
    /// Show information about a backup store
    Info {
        /// Store URI: /local/path, s3://bucket/key, gcs://bucket/key
        store: String,
    },
    /// Verify backup integrity
    Verify {
        /// Store URI: /local/path, s3://bucket/key, gcs://bucket/key
        store: String,
    },
    /// Migrate data between stores
    Migrate {
        /// Source store URI
        source: String,
        /// Destination store URI
        destination: String,
    },
    /// Run performance benchmarks
    Benchmark {
        /// Directory with corpus data for benchmarking
        corpus: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let opts = GlobalOpts {
        json: cli.json,
        verbose: cli.verbose,
        quiet: cli.quiet,
    };

    if !opts.verbose {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .init();
    }

    let result = match &cli.command {
        Commands::Backup {
            source,
            destination,
            chunk_size,
            similarity_threshold,
            force,
        } => backup::run_backup(source, destination, chunk_size, *similarity_threshold, *force, &opts),
        Commands::List { store } => list::run_list(store, &opts),
        Commands::Restore {
            source,
            destination,
            file,
        } => restore::run_restore(source, destination, file.as_deref(), &opts),
        Commands::Info { store } => info::run_info(store, &opts),
        Commands::Verify { store } => verify::run_verify(store, &opts),
        Commands::Migrate { source, destination } => migrate::run_migrate(source, destination, &opts),
        Commands::Benchmark { corpus } => {
            if !opts.quiet {
                println!("Benchmark not yet implemented for corpus: {}", corpus.display());
            }
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
