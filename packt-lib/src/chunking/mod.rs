pub mod fastcdc;
pub mod hints;

use crate::types::{Chunk, ChunkConfig};

/// Trait for content-defined chunking algorithms.
pub trait Chunker: Send + Sync {
    /// Split `data` into content-defined chunks.
    fn chunk(&self, data: &[u8]) -> Vec<Chunk>;

    /// Split `data` into chunks, preferring boundary positions from `hints`.
    ///
    /// The default implementation ignores hints and calls `chunk()` directly.
    /// Override this method in chunker implementations that support hint-guided
    /// boundary selection.
    fn chunk_with_hints(&self, data: &[u8], _hints: &[usize]) -> Vec<Chunk> {
        self.chunk(data)
    }

    /// Return the chunking configuration.
    fn config(&self) -> &ChunkConfig;
}
