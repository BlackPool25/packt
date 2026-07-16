use anyhow::{Context, Result};
use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::index::DedupIndex;
use packt_lib::index::hashindex::HashIndex;
use packt_lib::pipeline::{BackupPipeline, PipelineConfig};
use packt_lib::store::ContentStore;
use packt_lib::store::local::LocalStore;
use packt_lib::types::ChunkConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// File metadata entry stored in backup manifests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub size: u64,
    pub modified: String,
    pub permissions: u32,
    pub chunk_hashes: Vec<String>,
}

/// Get Unix permission bits from file metadata, or 0 on non-Unix platforms.
#[cfg(unix)]
fn get_unix_permissions(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

/// Get Unix permission bits from file metadata, or 0 on non-Unix platforms.
#[cfg(not(unix))]
fn get_unix_permissions(_metadata: &std::fs::Metadata) -> u32 {
    0
}

pub fn run_backup(source: &Path, destination: &Path, chunk_size: usize) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Source path does not exist: {}", source.display());
    }

    let config = ChunkConfig {
        min_size: (chunk_size / 2).max(64),
        avg_size: chunk_size,
        max_size: (chunk_size * 4).min(1_048_576),
    };

    if !config.validate() {
        anyhow::bail!("Invalid chunk configuration. Ensure min < avg < max and reasonable bounds.");
    }

    eprintln!("Opening store at: {}", destination.display());
    let store = Arc::new(LocalStore::open(destination).context("Failed to open local store")?);

    let index: Arc<dyn DedupIndex> = Arc::new(HashIndex::new(1_000_000));
    store
        .populate_index(&index)
        .context("Failed to populate index from store")?;
    let chunker = Arc::new(FastCdcChunker::new(config));
    let hasher = Arc::new(Blake3Hasher::new());

    let pipeline_config = PipelineConfig {
        chunk_config: config,
        ..Default::default()
    };

    let pipeline = BackupPipeline::new(
        pipeline_config,
        chunker,
        hasher,
        store as Arc<dyn ContentStore>,
        index.clone(),
    );

    let source_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let stats = pipeline
        .backup_file(source)
        .context("Backup pipeline failed")?;

    // Collect file metadata
    let metadata = std::fs::metadata(source).context("Failed to read source metadata")?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();

    let permissions = get_unix_permissions(&metadata);

    let manifest_entry = ManifestEntry {
        path: source_name.clone(),
        size: metadata.len(),
        modified,
        permissions,
        chunk_hashes: stats.chunk_hashes.iter().map(|h| h.to_hex()).collect(),
    };

    let manifests_dir = destination.join("manifests");
    std::fs::create_dir_all(&manifests_dir).context("Failed to create manifests directory")?;
    let manifest_path = manifests_dir.join(format!("{}.manifest", source_name));
    let manifest_json =
        serde_json::to_string_pretty(&manifest_entry).context("Failed to serialize manifest")?;
    std::fs::write(&manifest_path, &manifest_json).context("Failed to write manifest")?;

    println!("Backup complete:");
    println!("  File:            {}", source.display());
    println!("  Source size:     {} bytes", stats.source_size);
    println!("  Stored size:     {} bytes", stats.stored_size);
    println!(
        "  Dedup saved:     {} bytes ({:.1}%)",
        stats.dedup_size,
        stats.space_savings_pct()
    );
    println!("  Total chunks:    {}", stats.total_chunks);
    println!("  Unique chunks:   {}", stats.unique_chunks);
    println!("  Deduplicated:    {}", stats.dedup_chunks);
    println!("  Dedup ratio:     {:.2}x", stats.dedup_ratio());

    Ok(())
}
