use crate::index::DedupIndex;
use crate::store::ContentStore;
use crate::types::Hash;
use std::sync::Arc;

/// Dedup check stage. Queries the exact-dedup index before storage.
pub struct DedupStage {
    index: Arc<dyn DedupIndex>,
    _store: Arc<dyn ContentStore>,
}

impl DedupStage {
    /// Create a new dedup stage with the given index and store.
    /// `store` is retained for future use (e.g., populating index from existing data).
    pub fn new(index: Arc<dyn DedupIndex>, store: Arc<dyn ContentStore>) -> Self {
        Self { index, _store: store }
    }

    /// Check if a hash already exists in the dedup index.
    /// Returns `true` if this is a duplicate.
    pub fn check(&self, hash: &Hash) -> bool {
        self.index.contains(hash)
    }
}
