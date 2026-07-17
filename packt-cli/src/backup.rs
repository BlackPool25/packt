use anyhow::{Context, Result};
use packt_lib::store::{BackupOpts, Store};
use std::path::Path;

pub fn run_backup(source: &Path, destination: &str, chunk_size: usize, threshold: f64, force: bool) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Source not found: {}", source.display());
    }

    let config = Store::config_from_uri(destination).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let opts = BackupOpts {
        chunk_size,
        similarity_threshold: threshold,
        force,
    };

    let stats = store.backup(source, &opts).context("Failed to backup file")?;

    if stats.total_chunks == 0 {
        println!("Unchanged: {} (skipped)", source.display());
        return Ok(());
    }

    println!(
        "Backup: {} ({:.2}x ratio, {} chunks, {} near-dup, {}B delta-savings)",
        source.display(),
        stats.dedup_ratio(),
        stats.total_chunks,
        stats.near_duplicate_chunks,
        stats.delta_savings,
    );
    if stats.near_duplicate_chunks > 0 {
        println!("  Near-dup: {} chunks", stats.near_duplicate_chunks);
    }
    if stats.delta_compressed_chunks > 0 {
        println!(
            "  Delta: {} chunks ({} fallbacks), {} bytes saved",
            stats.delta_compressed_chunks, stats.delta_fallbacks, stats.delta_savings,
        );
    }

    Ok(())
}
