pub mod local;
pub mod pack;

use crate::error::Result;
use crate::types::{Hash, PackLocation};

/// Trait for content-addressed storage backends.
pub trait ContentStore: Send + Sync {
    /// Store a chunk, returning its location in the pack.
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation>;

    /// Retrieve chunk data by hash.
    fn get(&self, hash: &Hash) -> Result<Vec<u8>>;

    /// Check if a hash exists in the store.
    fn contains(&self, hash: &Hash) -> Result<bool>;

    /// Flush all pending writes to stable storage.
    fn flush(&self) -> Result<()>;
}
