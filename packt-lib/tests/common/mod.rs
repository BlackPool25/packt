use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::index::hashindex::HashIndex;
use packt_lib::pipeline::{BackupPipeline, PipelineConfig};
use packt_lib::store::local::LocalStore;
use packt_lib::types::ChunkConfig;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test environment with a store, index, chunker, hasher, and pipeline.
#[allow(unused_variables, dead_code)]
pub fn test_backup_env(
    corpus: &[u8],
    chunk_size: usize,
) -> (TempDir, BackupPipeline, Arc<packt_lib::store::local::LocalStore>) {
    let store_dir = TempDir::new().expect("Failed to create temp store dir");

    let config = ChunkConfig {
        min_size: (chunk_size / 2).max(64),
        avg_size: chunk_size,
        max_size: (chunk_size * 4).min(1_048_576),
    };

    let store = Arc::new(LocalStore::open(store_dir.path()).expect("Failed to open store"));
    let index = Arc::new(HashIndex::new(100_000));
    let chunker = Arc::new(FastCdcChunker::new(config));
    let hasher = Arc::new(Blake3Hasher::new());

    let pipeline = BackupPipeline::new(
        PipelineConfig::default(),
        chunker,
        hasher,
        store.clone() as Arc<dyn packt_lib::store::ContentStore>,
        index as Arc<dyn packt_lib::index::DedupIndex>,
    );

    (store_dir, pipeline, store)
}

#[allow(dead_code)]
pub fn blake3_file(path: &Path) -> String {
    let data = std::fs::read(path).expect("Failed to read file");
    let hash = blake3::hash(&data);
    hash.to_string()
}
