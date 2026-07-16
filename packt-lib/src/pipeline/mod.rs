pub mod dedup_stage;
pub mod hasher_stage;
use crate::chunking::Chunker;
use crate::error::{PacktError, Result};
use crate::hash::ContentHasher;
use crate::index::DedupIndex;
use crate::pipeline::dedup_stage::DedupStage;
use crate::pipeline::hasher_stage::HasherStage;
use crate::store::ContentStore;
use crate::types::{Chunk, ChunkConfig};

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

/// Configuration for the backup pipeline.
pub struct PipelineConfig {
    pub chunk_config: ChunkConfig,
    pub compression_level: i32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            chunk_config: ChunkConfig::default_32k(),
            compression_level: 3,
        }
    }
}

/// Backup pipeline orchestrator.
pub struct BackupPipeline {
    #[allow(dead_code)]
    config: PipelineConfig,
    hasher: Arc<dyn ContentHasher>,
    store: Arc<dyn ContentStore>,
    index: Arc<dyn DedupIndex>,
}

impl BackupPipeline {
    /// Create a new backup pipeline.
    ///
    /// `_chunker` is accepted for backward compatibility but the pipeline
    /// now uses its own streaming chunker internally.
    pub fn new(
        config: PipelineConfig,
        _chunker: Arc<dyn Chunker>,
        hasher: Arc<dyn ContentHasher>,
        store: Arc<dyn ContentStore>,
        index: Arc<dyn DedupIndex>,
    ) -> Self {
        Self {
            config,
            hasher,
            store,
            index,
        }
    }

    /// Run the backup pipeline on a single file using streaming CDC.
    ///
    /// Reads the file chunk-by-chunk via `fastcdc::v2020::StreamCDC` so the
    /// entire file is never loaded into memory at once.  Peak memory usage is
    /// roughly `max_chunk_size` (default 128 KiB) plus the bounded channel
    /// buffer (~8 MiB).
    ///
    /// # Errors
    /// Returns error if the file cannot be read, chunking fails, or the store
    /// encounters an I/O error.
    pub fn backup_file(&self, source: &Path) -> Result<BackupStats> {
        let hasher_stage = HasherStage::new(self.hasher.clone());
        let dedup_stage = DedupStage::new(self.index.clone(), self.store.clone());

        let mut stats = BackupStats::default();
        let config = self.config.chunk_config;

        // Get file size without reading contents
        stats.source_size = std::fs::metadata(source)
            .map_err(|e| PacktError::Io {
                context: format!("Failed to stat source: {}", source.display()),
                source: e,
            })?
            .len();

        // Open file for streaming
        let file = File::open(source).map_err(|e| PacktError::Io {
            context: format!("Failed to open source: {}", source.display()),
            source: e,
        })?;

        // Stream chunks via fastcdc's StreamCDC — internal buffer = max_size
        let chunker =
            fastcdc::v2020::StreamCDC::new(file, config.min_size, config.avg_size, config.max_size);

        let (dedup_tx, dedup_rx): (crossbeam_channel::Sender<DedupMessage>, _) =
            crossbeam_channel::bounded(64);

        let writer_handle = {
            let store = self.store.clone();
            let index = self.index.clone();
            std::thread::spawn(move || -> Result<WriterOutput> {
                let mut dedup_count = 0u64;
                let mut stored_count = 0u64;
                for msg in dedup_rx {
                    match msg {
                        DedupMessage::NewChunk { hash, data } => {
                            let loc = store.put(&hash, &data)?;
                            index.insert(hash, loc);
                            stored_count += 1;
                        }
                        DedupMessage::Duplicate { .. } => {
                            dedup_count += 1;
                        }
                    }
                }
                store.flush()?;
                Ok(WriterOutput {
                    dedup_count,
                    stored_count,
                })
            })
        };

        let mut chunk_hashes = Vec::new();
        let mut total_chunks = 0u64;

        for result in chunker {
            let chunk_data =
                result.map_err(|e| PacktError::Pipeline(format!("StreamCDC error: {e}")))?;

            let chunk = Chunk {
                offset: chunk_data.offset,
                length: chunk_data.length as u32,
                data: chunk_data.data,
            };
            total_chunks += 1;

            let chunk_len = u64::from(chunk.length);
            let hash = hasher_stage.hash(&chunk);
            chunk_hashes.push(hash);

            let is_dup = dedup_stage.check(&hash);

            if is_dup {
                let _ = dedup_tx.send(DedupMessage::Duplicate { hash, pack_id: 0 });
                stats.dedup_size += chunk_len;
            } else {
                let _ = dedup_tx.send(DedupMessage::NewChunk {
                    hash,
                    data: chunk.data,
                });
                stats.stored_size += chunk_len;
            }
        }

        // Drop senders to signal end of stream
        drop(dedup_tx);

        // Wait for writer
        let writer_output = writer_handle
            .join()
            .map_err(|e| PacktError::Pipeline(format!("Writer thread panicked: {e:?}")))??;

        stats.unique_chunks = writer_output.stored_count;
        stats.dedup_chunks = writer_output.dedup_count;
        stats.total_chunks = total_chunks;
        stats.chunk_hashes = chunk_hashes;

        Ok(stats)
    }
}

/// Messages sent to the writer stage.
pub enum DedupMessage {
    NewChunk {
        hash: crate::types::Hash,
        data: Vec<u8>,
    },
    Duplicate {
        hash: crate::types::Hash,
        pack_id: u32,
    },
}

struct WriterOutput {
    dedup_count: u64,
    stored_count: u64,
}

/// Statistics from a backup run.
#[derive(Debug, Clone, Default)]
pub struct BackupStats {
    pub source_size: u64,
    pub stored_size: u64,
    pub dedup_size: u64,
    pub total_chunks: u64,
    pub unique_chunks: u64,
    pub dedup_chunks: u64,
    /// Ordered list of chunk hashes for file reconstruction
    pub chunk_hashes: Vec<crate::types::Hash>,
}

impl BackupStats {
    /// Dedup ratio: source_size / stored_size
    #[must_use]
    pub fn dedup_ratio(&self) -> f64 {
        if self.stored_size == 0 {
            return 1.0;
        }
        self.source_size as f64 / self.stored_size as f64
    }

    /// Space savings as percentage
    #[must_use]
    pub fn space_savings_pct(&self) -> f64 {
        if self.source_size == 0 {
            return 0.0;
        }
        (1.0 - self.stored_size as f64 / self.source_size as f64) * 100.0
    }
}
