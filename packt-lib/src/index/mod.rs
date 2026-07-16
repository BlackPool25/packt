pub mod hashindex;

use crate::types::{Hash, PackLocation};

/// Trait for dedup index backends.
pub trait DedupIndex: Send + Sync {
    /// Insert a hash-location pair into the index.
    fn insert(&self, hash: Hash, location: PackLocation);

    /// Look up a hash and return its storage location, if present.
    fn lookup(&self, hash: &Hash) -> Option<PackLocation>;

    /// Check if a hash exists in the index.
    fn contains(&self, hash: &Hash) -> bool;

    /// Return the number of unique chunks in the index.
    fn len(&self) -> usize;

    /// Returns true if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
