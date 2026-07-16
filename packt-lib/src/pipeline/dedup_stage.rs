use crate::index::DedupIndex;
use crate::store::ContentStore;
use crate::types::Hash;
use std::sync::Arc;

pub struct DedupStage {
    index: Arc<dyn DedupIndex>,
    _store: Arc<dyn ContentStore>,
}

impl DedupStage {
    pub fn new(index: Arc<dyn DedupIndex>, store: Arc<dyn ContentStore>) -> Self {
        Self {
            index,
            _store: store,
        }
    }

    /// Check if a hash already exists in the dedup index.
    /// Returns `true` if this is a duplicate.
    pub fn check(&self, hash: &Hash) -> bool {
        self.index.contains(hash)
    }
}
