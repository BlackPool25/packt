pub mod delta;
pub mod local;
pub mod pack;

#[cfg(feature = "cloud")]
pub mod cloud;

#[cfg(feature = "cloud")]
pub use cloud::CloudStore;

pub mod store_api;
pub use store_api::{BackupOpts, FileInfo, Store, StoreConfig, StoreInfo, VerifyReport};

use crate::error::Result;
use crate::types::{Hash, PackLocation};

/// Trait for content-addressed storage backends.
pub trait ContentStore: Send + Sync {
    /// Store a chunk, returning its location in the pack.
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation>;

    /// Store a delta-compressed chunk, returning its location.
    ///
    /// `base_hash` identifies the base chunk used as the zstd dictionary.
    /// `delta_data` is the pre-compressed zstd dict frame.
    /// `orig_length` is the uncompressed size of the original chunk.
    fn put_delta(&self, hash: &Hash, base_hash: &Hash, delta_data: &[u8], orig_length: u32) -> Result<PackLocation>;

    /// Retrieve chunk data by hash.
    fn get(&self, hash: &Hash) -> Result<Vec<u8>>;

    /// Check if a hash exists in the store.
    fn contains(&self, hash: &Hash) -> Result<bool>;

    /// Attach a similarity signature to a chunk (for cross-session index rebuild).
    /// Must be called after `put()` or `put_delta()` for the same hash.
    fn put_signature(&self, hash: &Hash, signature: &[u8]) -> Result<()>;

    /// Flush all pending writes to stable storage.
    fn flush(&self) -> Result<()>;
}
