use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use packt_lib::store::{BackupOpts, Store};
use packt_lib::types::ChunkConfig;
use serde::Serialize;
use std::path::Path;

use crate::GlobalOpts;

/// Parse a chunk size argument.
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

#[derive(Serialize)]
struct Output {
    file: String,
    status: String,
    ratio: f64,
    chunks: u64,
    near_dup: u64,
    delta_savings: u64,
    peak_memory_bytes: u64,
}

pub fn run_backup(
    source: &Path,
    destination: &str,
    chunk_size_str: &str,
    threshold: f64,
    force: bool,
    gopts: &GlobalOpts,
) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Source not found: {}", source.display());
    }

    let config = Store::config_from_uri(destination).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let chunk_config = parse_chunk_config(chunk_size_str)
        .context("Invalid chunk size. Use a preset (8k, 32k, 64k) or raw byte count.")?;
    if !chunk_config.validate() {
        anyhow::bail!("Invalid chunk config: min/avg/max sizes violate constraints");
    }

    let pb = if !gopts.quiet && !gopts.json {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::with_template("{spinner:.green} {msg}").unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        bar.set_message(format!("Backing up {} ...", source.display()));
        bar.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(bar)
    } else {
        None
    };

    let opts = BackupOpts {
        chunk_config,
        similarity_threshold: threshold,
        force,
    };

    let stats = store.backup(source, &opts).context("Failed to backup file")?;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    if gopts.json {
        let out = if stats.total_chunks == 0 {
            Output {
                file: source.display().to_string(),
                status: "unchanged".into(),
                ratio: 1.0,
                chunks: 0,
                near_dup: 0,
                delta_savings: 0,
                peak_memory_bytes: 0,
            }
        } else {
            Output {
                file: source.display().to_string(),
                status: "done".into(),
                ratio: stats.dedup_ratio(),
                chunks: stats.total_chunks,
                near_dup: stats.near_duplicate_chunks,
                delta_savings: stats.delta_savings,
                peak_memory_bytes: stats.peak_memory_bytes,
            }
        };
        println!("{}", serde_json::to_string(&out)?);
    } else if !gopts.quiet {
        if stats.total_chunks == 0 {
            println!("Unchanged: {} (skipped)", source.display());
        } else {
            println!(
                "Backup: {} ({:.2}x ratio, {} chunks, {} near-dup, {}B delta-savings, peak mem: {})",
                source.display(),
                stats.dedup_ratio(),
                stats.total_chunks,
                stats.near_duplicate_chunks,
                delta_size_str(stats.delta_savings),
                byte_size_str(stats.peak_memory_bytes),
            );
            if stats.near_duplicate_chunks > 0 {
                println!("  Near-dup: {} chunks", stats.near_duplicate_chunks);
            }
            if stats.delta_compressed_chunks > 0 {
                println!(
                    "  Delta: {} chunks ({} fallbacks), {} saved",
                    stats.delta_compressed_chunks,
                    stats.delta_fallbacks,
                    delta_size_str(stats.delta_savings),
                );
            }
        }
    }

    Ok(())
}

fn byte_size_str(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn delta_size_str(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
