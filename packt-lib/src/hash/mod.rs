pub mod blake3_hasher;

use crate::types::{Chunk, Hash};

/// Trait for content hashing algorithms.
pub trait ContentHasher: Send + Sync {
    /// Compute the hash of a byte slice.
    fn hash(&self, data: &[u8]) -> Hash;

    /// Compute the hash of a chunk's data.
    fn hash_chunk(&self, chunk: &Chunk) -> Hash {
        self.hash(&chunk.data)
    }
}
