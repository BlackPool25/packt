use crate::hash::ContentHasher;
use crate::types::{Chunk, Hash};
use std::sync::Arc;

/// Hashes chunk data using the configured content hasher.
pub struct HasherStage {
    hasher: Arc<dyn ContentHasher>,
}

impl HasherStage {
    /// Create a new hasher stage with the given content hasher backend.
    pub fn new(hasher: Arc<dyn ContentHasher>) -> Self {
        Self { hasher }
    }

    /// Hash a single chunk and return its content hash.
    pub fn hash(&self, chunk: &Chunk) -> Hash {
        self.hasher.hash_chunk(chunk)
    }
}
