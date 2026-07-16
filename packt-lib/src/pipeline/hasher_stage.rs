use crate::hash::ContentHasher;
use crate::types::{Chunk, Hash};
use std::sync::Arc;

pub struct HasherStage {
    hasher: Arc<dyn ContentHasher>,
}

impl HasherStage {
    pub fn new(hasher: Arc<dyn ContentHasher>) -> Self {
        Self { hasher }
    }

    pub fn hash(&self, chunk: &Chunk) -> Hash {
        self.hasher.hash_chunk(chunk)
    }
}
