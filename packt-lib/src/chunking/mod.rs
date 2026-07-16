pub mod fastcdc;

use crate::types::{Chunk, ChunkConfig};

/// Trait for content-defined chunking algorithms.
pub trait Chunker: Send + Sync {
    /// Split `data` into content-defined chunks.
    fn chunk(&self, data: &[u8]) -> Vec<Chunk>;

    /// Return the chunking configuration.
    fn config(&self) -> &ChunkConfig;
}
