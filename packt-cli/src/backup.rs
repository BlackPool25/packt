use anyhow::{Context, Result};
use packt_lib::store::{BackupOpts, Store};
use packt_lib::types::ChunkConfig;
use std::path::Path;

/// Parse a chunk size argument.
///
/// Accepts human-readable presets:
/// - `8k` or `8K` → ChunkConfig::for_docker() (4KB/8KB/64KB)
/// - `32k` or `32K` → ChunkConfig::default_32k() (16KB/32KB/128KB)
/// - `64k` or `64K` → ChunkConfig { 32KB/64KB/256KB }
/// - A raw number (e.g., `16384`) → ChunkConfig with that avg_size
fn parse_chunk_config(s: &str) -> Option<ChunkConfig> {
    match s.to_lowercase().as_str() {
        "8k" => Some(ChunkConfig::for_docker()),
        "32k" => Some(ChunkConfig::default_32k()),
        "64k" => Some(ChunkConfig {
            min_size: 32_768,
            avg_size: 65_536,
            max_size: 262_144,
        }),
        _ => {
            let bytes: usize = s.parse().ok()?;
            Some(ChunkConfig {
                min_size: (bytes / 2).max(64),
                avg_size: bytes,
                max_size: (bytes * 4).min(1_048_576),
            })
        }
    }
}

pub fn run_backup(source: &Path, destination: &str, chunk_size_str: &str, threshold: f64, force: bool) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Source not found: {}", source.display());
    }

    let config = Store::config_from_uri(destination).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let chunk_config = parse_chunk_config(chunk_size_str)
        .context("Invalid chunk size. Use a preset (8k, 32k, 64k) or a raw byte count.")?;

    if !chunk_config.validate() {
        anyhow::bail!("Invalid chunk config: min/avg/max sizes violate constraints");
    }

    let opts = BackupOpts {
        chunk_config,
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
