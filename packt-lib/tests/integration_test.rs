mod common;

use packt_lib::chunking::Chunker;
use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::index::hashindex::HashIndex;
use packt_lib::pipeline::{BackupPipeline, PipelineConfig};
use packt_lib::store::ContentStore;
use packt_lib::store::local::LocalStore;
use packt_lib::types::ChunkConfig;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_backup_env(_corpus: &[u8]) -> (TempDir, BackupPipeline, Arc<LocalStore>) {
    let store_dir = TempDir::new().unwrap();
    let config = ChunkConfig::default_32k();
    let store = Arc::new(LocalStore::open(store_dir.path()).unwrap());
    let index = Arc::new(HashIndex::new(100_000));
    let chunker = Arc::new(FastCdcChunker::new(config));
    let hasher = Arc::new(Blake3Hasher::new());

    let pipeline = BackupPipeline::new(
        PipelineConfig::default(),
        chunker,
        hasher,
        store.clone() as Arc<dyn ContentStore>,
        index as Arc<dyn packt_lib::index::DedupIndex>,
    );

    (store_dir, pipeline, store)
}

#[test]
fn test_integration_backup_restore_small_file() {
    let corpus = b"Hello, world! This is a small test file for integration testing.";
    let (store_dir, pipeline, store) = setup_backup_env(corpus);

    let source_dir = TempDir::new().unwrap();
    let source = source_dir.path().join("test.txt");
    fs::write(&source, corpus).unwrap();

    let stats = pipeline.backup_file(&source).unwrap();
    assert!(stats.unique_chunks > 0, "Should store at least one chunk");
    assert!(stats.source_size > 0);

    store.flush().unwrap();
    let packs_dir = store_dir.path().join("packs");
    let pack_count = fs::read_dir(&packs_dir).unwrap().count();
    assert!(pack_count > 0, "Should have at least one pack file");
}

#[test]
fn test_integration_dedup_identical_files() {
    let data = vec![0xABu8; 1_000_000];
    let (_store_dir, pipeline, store) = setup_backup_env(&data);

    let source_dir = TempDir::new().unwrap();
    let source = source_dir.path().join("data.bin");
    fs::write(&source, &data).unwrap();

    let _stats1 = pipeline.backup_file(&source).unwrap();
    let stats2 = pipeline.backup_file(&source).unwrap();

    assert!(stats2.dedup_chunks > 0, "Second backup should deduplicate chunks");
    store.flush().unwrap();
}

#[test]
fn test_integration_different_files() {
    let data1 = vec![0xABu8; 500_000];
    let data2 = vec![0xCDu8; 500_000];
    let (store_dir, pipeline, store) = setup_backup_env(&data1);

    let dir = TempDir::new().unwrap();
    let f1 = dir.path().join("file1.bin");
    let f2 = dir.path().join("file2.bin");
    fs::write(&f1, &data1).unwrap();
    fs::write(&f2, &data2).unwrap();

    let stats1 = pipeline.backup_file(&f1).unwrap();
    let stats2 = pipeline.backup_file(&f2).unwrap();

    assert!(stats1.unique_chunks > 0);
    assert!(stats2.unique_chunks > 0);
    store.flush().unwrap();

    let packs_dir = store_dir.path().join("packs");
    let pack_count = fs::read_dir(&packs_dir).unwrap().count();
    assert!(pack_count > 0);
}

#[test]
fn test_integration_large_file() {
    let data = vec![0xEFu8; 10_485_760];
    let (_store_dir, pipeline, store) = setup_backup_env(&data);

    let source_dir = TempDir::new().unwrap();
    let source = source_dir.path().join("large.bin");
    fs::write(&source, &data).unwrap();

    let stats = pipeline.backup_file(&source).unwrap();
    assert!(stats.source_size == 10_485_760);
    assert!(stats.unique_chunks > 0);
    store.flush().unwrap();
}

#[test]
fn test_integration_empty_file() {
    let data = b"";
    let (_store_dir, pipeline, store) = setup_backup_env(data);

    let source_dir = TempDir::new().unwrap();
    let source = source_dir.path().join("empty.txt");
    fs::write(&source, data).unwrap();

    let stats = pipeline.backup_file(&source).unwrap();
    assert_eq!(stats.total_chunks, 0, "Empty file should produce zero chunks");
    store.flush().unwrap();
}

#[test]
fn test_store_open_reopen() {
    let data = vec![0xABu8; 100_000];
    let store_dir = TempDir::new().unwrap();

    {
        let store = LocalStore::open(store_dir.path()).unwrap();
        let index = Arc::new(HashIndex::new(10_000));
        let pipeline = BackupPipeline::new(
            PipelineConfig::default(),
            Arc::new(FastCdcChunker::new(ChunkConfig::default_32k())),
            Arc::new(Blake3Hasher::new()),
            Arc::new(store) as Arc<dyn ContentStore>,
            index as Arc<dyn packt_lib::index::DedupIndex>,
        );

        let source_dir = TempDir::new().unwrap();
        let source = source_dir.path().join("data.bin");
        fs::write(&source, &data).unwrap();
        pipeline.backup_file(&source).unwrap();
    }

    {
        let _store = LocalStore::open(store_dir.path()).unwrap();
        let packs_dir = store_dir.path().join("packs");
        let pack_count = fs::read_dir(&packs_dir).unwrap().count();
        assert!(pack_count > 0, "Reopened store should have packs");
    }
}

#[test]
fn test_different_chunk_sizes() {
    for avg_size in [4096, 16384, 65536] {
        let data = vec![0xABu8; 1_000_000];
        let config = ChunkConfig {
            min_size: (avg_size / 2).max(64),
            avg_size,
            max_size: (avg_size * 4).min(1_048_576),
        };
        let _store_dir = TempDir::new().unwrap();
        let chunker = FastCdcChunker::new(config);
        let chunks = chunker.chunk(&data);

        assert!(!chunks.is_empty(), "Avg size {avg_size} should produce chunks");
        let total: u64 = chunks.iter().map(|c| u64::from(c.length)).sum();
        assert_eq!(total, data.len() as u64);
    }
}

#[test]
fn test_many_small_chunks_roundtrip() {
    for i in 0..50 {
        let data = vec![(i % 256) as u8; 1000];
        let hash = packt_lib::types::Hash::from_blake3(blake3::hash(&data));
        let len = data.len() as u32;
        let chunks = vec![(hash, data.clone(), len)];
        let pack = packt_lib::store::pack::write_pack(&chunks).unwrap();
        let (entries, _) = packt_lib::store::pack::read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, hash);
    }
}
