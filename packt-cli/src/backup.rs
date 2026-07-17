use anyhow::Result;
use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::index::DedupIndex;
use packt_lib::index::hashindex::HashIndex;
use packt_lib::pipeline::{BackupPipeline, PipelineConfig};
use packt_lib::similarity::SimilarityConfig;
use packt_lib::store::ContentStore;
use packt_lib::store::local::LocalStore;
use packt_lib::types::ChunkConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub size: u64,
    pub modified: String,
    pub permissions: u32,
    pub chunk_hashes: Vec<String>,
}

#[cfg(unix)]
fn perm(m: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}
#[cfg(not(unix))]
fn perm(_: &std::fs::Metadata) -> u32 {
    0
}

pub fn run_backup(source: &Path, destination: &Path, chunk_size: usize, threshold: f64) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Source not found: {}", source.display());
    }
    let cfg = ChunkConfig {
        min_size: (chunk_size / 2).max(64),
        avg_size: chunk_size,
        max_size: (chunk_size * 4).min(1_048_576),
    };
    if !cfg.validate() {
        anyhow::bail!("Invalid chunk config");
    }
    let local_store = Arc::new(LocalStore::open(destination)?);
    let store: Arc<dyn ContentStore> = local_store.clone();
    let index: Arc<dyn DedupIndex> = Arc::new(HashIndex::new(1_000_000));
    local_store.populate_index(&index)?;
    local_store.set_index(index.clone());
    let sim = if threshold > 0.0 {
        Some(SimilarityConfig {
            threshold: threshold.clamp(0.0, 1.0),
            ..Default::default()
        })
    } else {
        None
    };
    let pipeline = BackupPipeline::new(
        PipelineConfig {
            chunk_config: cfg,
            similarity_config: sim,
            ..Default::default()
        },
        Arc::new(FastCdcChunker::new(cfg)),
        Arc::new(Blake3Hasher::new()),
        store.clone(),
        index.clone(),
    );

    // Rebuild similarity index from stored signatures for cross-session near-dup detection
    if let Some(sim_stage) = pipeline.similarity() {
        use packt_lib::similarity::palantir::PalantirIndex;
        let mut palantir = PalantirIndex::new(1_000_000);
        if local_store.rebuild_similarity_index(&mut palantir).is_ok() {
            sim_stage.rebuild_index(palantir.export_entries());
        }
    }

    let stats = pipeline.backup_file(source)?;
    let meta = std::fs::metadata(source)?;
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let entry = ManifestEntry {
        path: name,
        size: meta.len(),
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default(),
        permissions: perm(&meta),
        chunk_hashes: stats.chunk_hashes.iter().map(|h| h.to_hex()).collect(),
    };
    let md = destination.join("manifests");
    std::fs::create_dir_all(&md)?;
    std::fs::write(
        md.join(format!("{}.manifest", source.file_name().unwrap().to_string_lossy())),
        serde_json::to_string_pretty(&entry)?,
    )?;
    println!(
        "Backup: {} ({:.2}x ratio, {} chunks, {} near-dup, {}B delta-savings)",
        source.display(),
        stats.dedup_ratio(),
        stats.total_chunks,
        stats.near_duplicate_chunks,
        stats.delta_savings,
    );
    if pipeline.has_similarity() {
        println!("  Sim index: {} entries", stats.similarity_index_size);
    }
    if stats.delta_compressed_chunks > 0 {
        println!(
            "  Delta: {} chunks ({} fallbacks), {} bytes saved ({:.1} avg)",
            stats.delta_compressed_chunks,
            stats.delta_fallbacks,
            stats.delta_savings,
            if stats.delta_compressed_chunks > 0 {
                stats.delta_savings as f64 / stats.delta_compressed_chunks as f64
            } else {
                0.0
            }
        );
    }
    Ok(())
}
