use crate::error::{PacktError, Result};
use crate::index::DedupIndex;
use crate::store::ContentStore;
use crate::store::pack;
use crate::types::{Hash, PackLocation};
use lru::LruCache;
use opendal::blocking;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

/// CloudStore implements ContentStore for S3, GCS (and any OpenDAL-supported backend).
///
/// Layout in bucket:
///   packs/{pack_id:08}.pack   — Pack files (default 16 MB each)
///   packs/_meta.index          — Global index mapping chunk hash → pack location
///                                (compact binary, rewritten on each flush)
///
/// Index population on open:
///   1. Download `packs/_meta.index` if it exists → fast path
///   2. Fallback: list `*.pack` objects, download footers, rebuild index
///
/// No encryption. No GC. Pack format is PACKv3 (same as LocalStore).
#[cfg(feature = "cloud")]
pub struct CloudStore {
    /// Tokio runtime kept alive for `blocking::Operator`.
    _rt: Runtime,
    operator: blocking::Operator,
    state: Mutex<CloudState>,
    pack_target_size: u64,
    next_pack_id: AtomicU32,
    index: Mutex<Option<Arc<dyn DedupIndex>>>,
    /// Optional local disk cache for downloaded packs.
    cache: Option<Mutex<LruCache<u32, ()>>>,
    cache_dir: Option<PathBuf>,
}

#[cfg(feature = "cloud")]
struct CloudState {
    pending_chunks: Vec<PendingEntry>,
    pending_size: u64,
    packs: HashMap<u32, PackMetadata>,
}

#[cfg(feature = "cloud")]
struct PendingEntry {
    hash: Hash,
    data: Vec<u8>,
    orig_length: u32,
    entry_type: pack::EntryType,
    signature: Option<Vec<u8>>,
}

#[cfg(feature = "cloud")]
#[derive(Default)]
struct PackMetadata {
    entries: Vec<pack::IndexEntry>,
    superblock: Option<Vec<u8>>,
}

/// Entry in the global `_meta.index` file.
#[cfg(feature = "cloud")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetaIndexEntry {
    hash: [u8; 32],
    pack_id: u32,
    offset: u64,
    length: u32,
    orig_length: u32,
}

#[cfg(feature = "cloud")]
const PACK_DIR: &str = "packs/";
#[cfg(feature = "cloud")]
const META_INDEX_PATH: &str = "packs/_meta.index";
#[cfg(feature = "cloud")]
const DEFAULT_PACK_TARGET: u64 = 16 * 1024 * 1024; // 16 MB
const DEFAULT_CACHE_CAPACITY: usize = 128; // max 128 packs = ~2GB at 16MB each

#[cfg(feature = "cloud")]
impl CloudStore {
    /// Open a CloudStore backed by the given OpenDAL operator.
    ///
    /// The operator must have retry, timeout, and other middleware configured
    /// by the caller before being passed here.
    ///
    /// If the bucket already contains packs, the index is populated from
    /// `_meta.index` (fast path) or by scanning pack footers (fallback).
    #[allow(clippy::needless_pass_by_value)]
    pub fn open(operator: opendal::Operator, index: Arc<dyn DedupIndex>, cache_dir: Option<PathBuf>) -> Result<Self> {
        let rt = Runtime::new()
            .map_err(|e| PacktError::Config(format!("failed to create tokio runtime for CloudStore: {e}")))?;
        let guard = rt.enter();
        let bop = blocking::Operator::new(operator).map_err(|e| PacktError::Cloud {
            context: "failed to create blocking OpenDAL operator".into(),
            source: e,
        })?;
        drop(guard);

        let cache = cache_dir.as_ref().map(|dir| {
            let cache_packs = dir.join("packs");
            std::fs::create_dir_all(&cache_packs).ok();
            Mutex::new(LruCache::new(NonZeroUsize::new(DEFAULT_CACHE_CAPACITY).unwrap()))
        });

        let store = Self {
            _rt: rt,
            operator: bop,
            state: Mutex::new(CloudState {
                pending_chunks: Vec::new(),
                pending_size: 0,
                packs: HashMap::new(),
            }),
            pack_target_size: DEFAULT_PACK_TARGET,
            next_pack_id: AtomicU32::new(0),
            index: Mutex::new(Some(index.clone())),
            cache,
            cache_dir,
        };

        // Populate index from existing packs
        store.populate_index(&index)?;

        Ok(store)
    }

    /// Populate the DedupIndex from existing packs in the bucket.
    fn populate_index(&self, index: &Arc<dyn DedupIndex>) -> Result<()> {
        // Fast path: try reading _meta.index
        if let Ok(meta_buf) = self.operator.read(META_INDEX_PATH) {
            let meta_bytes = meta_buf.to_vec();
            if let Ok(entries) = postcard::take_from_bytes::<Vec<MetaIndexEntry>>(&meta_bytes) {
                let mut max_pack_id = 0u32;
                for entry in entries.0 {
                    let hash = Hash::from_bytes(entry.hash);
                    let loc = PackLocation {
                        pack_id: entry.pack_id,
                        offset: entry.offset,
                        length: entry.length,
                        orig_length: entry.orig_length,
                    };
                    // Index expects (hash, location) — push directly
                    index.insert(hash, loc);
                    max_pack_id = max_pack_id.max(entry.pack_id);
                }
                self.next_pack_id.store(max_pack_id + 1, Ordering::SeqCst);
                return Ok(());
            }
        }

        // Fallback: iterate pack files, download footers, rebuild index
        let mut pack_ids: Vec<u32> = Vec::new();
        let lister = self.operator.lister(PACK_DIR).map_err(|e| PacktError::Cloud {
            context: "failed to list packs directory".into(),
            source: e,
        })?;

        for result in lister {
            let entry = result.map_err(|e| PacktError::Cloud {
                context: "failed to iterate packs".into(),
                source: e,
            })?;
            let path = entry.path().to_string();
            if let Some(stem) = path.strip_prefix(PACK_DIR).and_then(|s| s.strip_suffix(".pack")) {
                if let Ok(id) = stem.parse::<u32>() {
                    pack_ids.push(id);
                }
            }
        }

        pack_ids.sort_unstable();

        let mut max_pack_id = 0u32;
        for &pack_id in &pack_ids {
            let pack_path = format!("{PACK_DIR}{pack_id:08}.pack");
            let pack_buf = self.operator.read(&pack_path).map_err(|e| PacktError::Cloud {
                context: format!("failed to read pack {pack_id}"),
                source: e,
            })?;
            let pack_data = pack_buf.to_vec();

            match pack::read_pack(&pack_data) {
                Ok((entries, _checksum, superblock)) => {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
                    state.packs.insert(
                        pack_id,
                        PackMetadata {
                            entries: entries.clone(),
                            superblock,
                        },
                    );
                    drop(state);

                    for entry in &entries {
                        let loc = PackLocation {
                            pack_id,
                            offset: entry.offset,
                            length: entry.length,
                            orig_length: entry.orig_length,
                        };
                        index.insert(entry.hash, loc);
                    }
                    max_pack_id = max_pack_id.max(pack_id);
                }
                Err(e) => {
                    return Err(PacktError::StoreCorrupted(format!("pack {pack_id} is corrupted: {e}")));
                }
            }
        }

        self.next_pack_id.store(max_pack_id + 1, Ordering::SeqCst);
        Ok(())
    }

    /// Set the dedup index for O(1) lookups.
    pub fn set_index(&self, index: Arc<dyn DedupIndex>) {
        if let Ok(mut guard) = self.index.lock() {
            *guard = Some(index);
        }
    }

    /// Phase 1: drain pending chunks under lock, return entries for I/O.
    fn flush_prepare(state: &mut CloudState) -> Vec<pack::PackEntry> {
        let chunks: Vec<_> = state
            .pending_chunks
            .drain(..)
            .map(|e| pack::PackEntry {
                hash: e.hash,
                data: e.data,
                orig_length: e.orig_length,
                entry_type: e.entry_type,
                signature: e.signature,
            })
            .collect();
        state.pending_size = 0;
        chunks
    }

    /// Phase 2: write pack bytes to cloud (no lock held).
    /// Optionally writes to local cache when `cache_path` is provided.
    fn flush_write(
        chunks: &[pack::PackEntry],
        pack_id: u32,
        operator: &blocking::Operator,
        cache_path: Option<&Path>,
    ) -> Result<PackMetadata> {
        if chunks.is_empty() {
            return Ok(PackMetadata::default());
        }

        let pack_bytes = pack::write_pack(chunks)?;
        let pack_path = format!("{PACK_DIR}{pack_id:08}.pack");

        operator
            .write(&pack_path, pack_bytes.clone())
            .map_err(|e| PacktError::Cloud {
                context: format!("failed to upload pack {pack_id}"),
                source: e,
            })?;

        if let Some(cache_dir) = cache_path {
            let cache_pack_path = cache_dir.join(format!("{pack_id:08}.pack"));
            std::fs::write(&cache_pack_path, &pack_bytes).ok();
        }

        match pack::read_pack(&pack_bytes) {
            Ok((entries, _checksum, superblock)) => Ok(PackMetadata { entries, superblock }),
            Err(e) => Err(PacktError::StoreCorrupted(format!(
                "newly-constructed pack {pack_id} failed verification: {e}"
            ))),
        }
    }

    /// Write or rewrite the global `_meta.index` for fast reopen.
    fn write_meta_index(state: &CloudState, operator: &blocking::Operator) -> Result<()> {
        let mut entries: Vec<MetaIndexEntry> = Vec::new();
        for (&pack_id, pack) in &state.packs {
            for pe in &pack.entries {
                entries.push(MetaIndexEntry {
                    hash: pe.hash.0,
                    pack_id,
                    offset: pe.offset,
                    length: pe.length,
                    orig_length: pe.orig_length,
                });
            }
        }

        if entries.is_empty() {
            // No packs → delete meta index if it exists
            let _ = operator.delete(META_INDEX_PATH);
            return Ok(());
        }

        let data = postcard::to_stdvec(&entries)
            .map_err(|e| PacktError::Serialization(format!("meta index serialization: {e}")))?;

        operator.write(META_INDEX_PATH, data).map_err(|e| PacktError::Cloud {
            context: "failed to write _meta.index".into(),
            source: e,
        })?;

        Ok(())
    }

    /// Read pack bytes, checking local cache first if enabled.
    fn read_pack_cached(&self, pack_id: u32) -> Result<(Vec<u8>, bool)> {
        let pack_path = format!("{PACK_DIR}{pack_id:08}.pack");

        // Check local cache first
        if let Some(ref cache) = self.cache {
            let cache_pack_path = self
                .cache_dir
                .as_ref()
                .unwrap()
                .join("packs")
                .join(format!("{pack_id:08}.pack"));
            if cache_pack_path.exists() {
                if let Ok(data) = std::fs::read(&cache_pack_path) {
                    // Promote in LRU
                    if let Ok(mut guard) = cache.lock() {
                        guard.put(pack_id, ());
                    }
                    return Ok((data, true));
                }
            }
        }

        // Download from cloud
        let pack_buf = self.operator.read(&pack_path).map_err(|e| PacktError::Cloud {
            context: format!("failed to download pack {pack_id}"),
            source: e,
        })?;
        let data = pack_buf.to_vec();

        // Write to cache
        if let Some(ref cache) = self.cache {
            let cache_pack_path = self
                .cache_dir
                .as_ref()
                .unwrap()
                .join("packs")
                .join(format!("{pack_id:08}.pack"));
            if let Ok(mut guard) = cache.lock() {
                guard.put(pack_id, ());
            }
            std::fs::write(&cache_pack_path, &data).ok();
        }

        Ok((data, false))
    }

    /// O(1) lookup via DedupIndex.
    fn lookup_index(&self, hash: &Hash) -> Option<PackLocation> {
        if let Ok(guard) = self.index.lock() {
            if let Some(ref idx) = *guard {
                return idx.lookup(hash);
            }
        }
        None
    }

    /// Internal get: checks pending, index, then cloud.
    fn get_inner(&self, hash: &Hash, depth: usize) -> Result<Vec<u8>> {
        if depth > 16 {
            return Err(PacktError::InvalidPackFormat(
                "delta chain too deep (possible cycle)".into(),
            ));
        }

        // Phase 1: Check pending chunks first (under lock, but pending is small).
        {
            let state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
            for entry in &state.pending_chunks {
                if entry.hash == *hash {
                    return Ok(entry.data.clone());
                }
            }
        }

        // Phase 2: Try the dedup index for O(1) lookup.
        // NOTE: If the index entry is stale (placeholder offset=0 after
        // flush), fall through to linear scan instead of erroring.
        let loc = self.lookup_index(hash);
        if let Some(loc) = loc {
            if let Ok(data) = self.read_from_location(hash, loc, depth) {
                return Ok(data);
            }
        }

        // Phase 3: Fall back to linear scan of packs and pending.
        let match_info = {
            let state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;

            let mut info = None;
            for (pack_id, pack) in &state.packs {
                for entry in &pack.entries {
                    if entry.hash == *hash {
                        let loc = PackLocation {
                            pack_id: *pack_id,
                            offset: entry.offset,
                            length: entry.length,
                            orig_length: entry.orig_length,
                        };
                        let (base_hash, needs_raw) = match &entry.entry_type {
                            pack::EntryType::Delta { base_hash } => (Some(*base_hash), false),
                            pack::EntryType::Full => (None, false),
                            pack::EntryType::FullRaw => (None, true),
                        };
                        info = Some((loc, base_hash, needs_raw, pack.superblock.clone()));
                        break;
                    }
                }
                if info.is_some() {
                    break;
                }
            }
            info
        };

        let Some((loc, _base_hash, _needs_raw, _superblock)) = match_info else {
            return Err(PacktError::ChunkNotFound(hash.to_hex()));
        };

        self.read_from_location(hash, loc, depth)
    }

    /// Fast path: read chunk by PackLocation from index lookup.
    ///
    /// Downloads the pack from cloud storage, parses its entries to find the
    /// matching chunk, then extracts and verifies. Does NOT rely on
    /// `state.packs` — works even when index was populated from `_meta.index`
    /// without loading pack metadata into memory.
    fn read_from_location(&self, hash: &Hash, loc: PackLocation, depth: usize) -> Result<Vec<u8>> {
        let (pack_data, _from_cache) = self.read_pack_cached(loc.pack_id)?;

        let (entries, _checksum, superblock) = pack::read_pack(&pack_data)
            .map_err(|e| PacktError::StoreCorrupted(format!("pack {} corrupted: {e}", loc.pack_id)))?;

        let entry = entries
            .iter()
            .find(|e| e.offset == loc.offset && e.length == loc.length)
            .ok_or_else(|| PacktError::ChunkNotFound(hash.to_hex()))?;

        let (base_hash, needs_raw) = match &entry.entry_type {
            pack::EntryType::Delta { base_hash } => (Some(*base_hash), false),
            pack::EntryType::Full => (None, false),
            pack::EntryType::FullRaw => (None, true),
        };

        self.extract_chunk(
            hash,
            &pack_data,
            &loc,
            base_hash,
            needs_raw,
            superblock.as_deref(),
            depth,
        )
    }

    /// Chunk data extractor: decompress from + verify BLAKE3.
    #[allow(clippy::too_many_arguments)]
    fn extract_chunk(
        &self,
        hash: &Hash,
        pack_data: &[u8],
        loc: &PackLocation,
        base_hash: Option<Hash>,
        needs_raw: bool,
        superblock: Option<&[u8]>,
        depth: usize,
    ) -> Result<Vec<u8>> {
        let stored_data = if let Some(base_hash) = base_hash {
            let base_chunk = self.get_inner(&base_hash, depth + 1)?;
            pack::read_delta_chunk(pack_data, loc, &base_chunk)?
        } else if let Some(sb) = superblock {
            let start = loc.offset as usize;
            let end = start + loc.length as usize;
            if end > sb.len() {
                return Err(PacktError::InvalidPackFormat(format!(
                    "chunk {}+{} exceeds superblock size {}",
                    loc.offset,
                    loc.length,
                    sb.len()
                )));
            }
            let raw = &sb[start..end];
            if needs_raw {
                raw.to_vec()
            } else {
                zstd::bulk::decompress(raw, loc.orig_length as usize)
                    .map_err(|e| PacktError::Serialization(format!("zstd decompress: {e}")))?
            }
        } else if needs_raw {
            pack::read_raw_chunk(pack_data, loc)?
        } else {
            pack::read_chunk(pack_data, loc)?
        };

        let actual_hash = blake3::hash(&stored_data);
        if Hash::from_blake3(actual_hash) != *hash {
            return Err(PacktError::ChecksumMismatch {
                expected: hash.to_hex(),
                actual: Hash::from_blake3(actual_hash).to_hex(),
            });
        }

        Ok(stored_data)
    }
}

#[cfg(feature = "cloud")]
impl ContentStore for CloudStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation> {
        // Phase 1: DedupIndex O(1) check
        if let Some(loc) = self.lookup_index(hash) {
            return Ok(loc);
        }

        // Phase 2: Check pending chunks
        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(PackLocation {
                    pack_id: self.next_pack_id.load(Ordering::SeqCst),
                    offset: 0,
                    length: 0,
                    orig_length: entry.orig_length,
                });
            }
        }

        // Phase 3: Append to pending buffer
        let orig_length = data.len() as u32;
        state.pending_chunks.push(PendingEntry {
            hash: *hash,
            data: data.to_vec(),
            orig_length,
            entry_type: pack::EntryType::Full,
            signature: None,
        });
        state.pending_size += data.len() as u64;

        if state.pending_size >= self.pack_target_size {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            let chunks = Self::flush_prepare(&mut state);
            let operator = self.operator.clone();
            let cache_path = self.cache_dir.as_ref().map(|d| d.join("packs"));
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &operator, cache_path.as_deref())?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
            let _ = Self::write_meta_index(&state, &self.operator);
        }

        Ok(PackLocation {
            pack_id: self.next_pack_id.load(Ordering::SeqCst),
            offset: 0,
            length: 0,
            orig_length,
        })
    }

    fn put_delta(&self, hash: &Hash, base_hash: &Hash, delta_data: &[u8], orig_length: u32) -> Result<PackLocation> {
        if let Some(loc) = self.lookup_index(hash) {
            return Ok(loc);
        }

        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(PackLocation {
                    pack_id: self.next_pack_id.load(Ordering::SeqCst),
                    offset: 0,
                    length: 0,
                    orig_length: entry.orig_length,
                });
            }
        }

        state.pending_chunks.push(PendingEntry {
            hash: *hash,
            data: delta_data.to_vec(),
            orig_length,
            entry_type: pack::EntryType::Delta { base_hash: *base_hash },
            signature: None,
        });
        state.pending_size += delta_data.len() as u64;

        if state.pending_size >= self.pack_target_size {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            let chunks = Self::flush_prepare(&mut state);
            let operator = self.operator.clone();
            let cache_path = self.cache_dir.as_ref().map(|d| d.join("packs"));
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &operator, cache_path.as_deref())?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
            let _ = Self::write_meta_index(&state, &self.operator);
        }

        Ok(PackLocation {
            pack_id: self.next_pack_id.load(Ordering::SeqCst),
            offset: 0,
            length: 0,
            orig_length,
        })
    }

    fn put_signature(&self, hash: &Hash, signature: &[u8]) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
        for entry in &mut state.pending_chunks {
            if entry.hash == *hash {
                entry.signature = Some(signature.to_vec());
                return Ok(());
            }
        }
        Err(PacktError::ChunkNotFound(hash.to_hex()))
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        self.get_inner(hash, 0)
    }

    fn contains(&self, hash: &Hash) -> Result<bool> {
        if self.lookup_index(hash).is_some() {
            return Ok(true);
        }
        let state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(true);
            }
        }
        for pack in state.packs.values() {
            for entry in &pack.entries {
                if entry.hash == *hash {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn flush(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
        if !state.pending_chunks.is_empty() {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            let chunks = Self::flush_prepare(&mut state);
            let operator = self.operator.clone();
            let cache_path = self.cache_dir.as_ref().map(|d| d.join("packs"));
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &operator, cache_path.as_deref())?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("cloud store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
            Self::write_meta_index(&state, &self.operator)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "cloud")]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::index::hashindex::HashIndex;
    use crate::store::ContentStore;
    use opendal::Operator;
    use opendal::services;
    use std::sync::Arc;

    fn setup_cloud_store() -> Result<(Arc<HashIndex>, CloudStore)> {
        let op = Operator::new(services::Memory::default())
            .map_err(|e| PacktError::Cloud {
                context: "failed to create memory operator".into(),
                source: e,
            })?
            .finish();
        let index = Arc::new(HashIndex::new(1_000_000));
        let store = CloudStore::open(op, index.clone(), None)?;
        Ok((index, store))
    }

    #[test]
    fn test_cloud_store_roundtrip() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let data = b"hello world this is a test chunk".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let _loc = store.put(&hash, &data)?;
        store.flush()?;
        let retrieved = store.get(&hash)?;
        assert_eq!(retrieved, data);
        Ok(())
    }

    #[test]
    fn test_cloud_store_dedup() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let data = b"dedup test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;
        store.put(&hash, &data)?;
        store.flush()?;
        Ok(())
    }

    #[test]
    fn test_cloud_store_missing() {
        let (_index, store) = setup_cloud_store().unwrap();
        let hash = Hash::from_bytes([0u8; 32]);
        let result = store.get(&hash);
        assert!(result.is_err(), "getting nonexistent chunk should fail");
    }

    #[test]
    fn test_cloud_store_contains() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let data = b"test contains".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        assert!(!store.contains(&hash)?);
        store.put(&hash, &data)?;
        store.flush()?;
        assert!(store.contains(&hash)?);
        Ok(())
    }

    #[test]
    fn test_cloud_store_large_chunk() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let data = vec![0xABu8; 1_000_000];
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;
        let retrieved = store.get(&hash)?;
        assert_eq!(retrieved.len(), data.len());
        assert_eq!(retrieved, data);
        Ok(())
    }

    #[test]
    fn test_cloud_store_integrity() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let data = b"integrity check data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;
        let retrieved = store.get(&hash)?;
        let verify = blake3::hash(&retrieved);
        assert_eq!(Hash::from_blake3(verify), hash);
        Ok(())
    }

    #[test]
    fn test_cloud_store_delta_roundtrip() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let base_data: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let base_hash = Hash::from_blake3(blake3::hash(&base_data));
        store.put(&base_hash, &base_data)?;
        store.flush()?;

        let mut target_data = base_data.clone();
        target_data[100] = 0xFF;
        let target_hash = Hash::from_blake3(blake3::hash(&target_data));

        let encoder = crate::store::delta::DeltaEncoder::new(3);
        let delta = encoder
            .try_encode(&base_data, &target_data)?
            .expect("delta should be beneficial for similar data");

        let _loc = store.put_delta(&target_hash, &base_hash, &delta, target_data.len() as u32)?;
        store.flush()?;

        let retrieved = store.get(&target_hash)?;
        assert_eq!(retrieved, target_data, "delta roundtrip should reconstruct original");
        Ok(())
    }

    #[test]
    fn test_cloud_store_multiple_chunks() -> Result<()> {
        let (_index, store) = setup_cloud_store()?;
        let chunks: Vec<_> = (0..50)
            .map(|i| {
                let data = format!("chunk number {i} with some data to make it unique");
                let hash = Hash::from_blake3(blake3::hash(data.as_bytes()));
                (hash, data.into_bytes())
            })
            .collect();

        for (hash, data) in &chunks {
            store.put(hash, data)?;
        }
        store.flush()?;

        for (hash, data) in &chunks {
            let retrieved = store.get(hash)?;
            assert_eq!(&retrieved, data);
        }
        Ok(())
    }

    #[test]
    fn test_cloud_store_index_persistence() -> Result<()> {
        // Test that _meta.index allows reopening with populated index
        let op = Operator::new(services::Memory::default())
            .map_err(|e| PacktError::Cloud {
                context: "failed to create memory operator".into(),
                source: e,
            })?
            .finish();

        {
            let index = Arc::new(HashIndex::new(1_000_000));
            let store = CloudStore::open(op.clone(), index.clone(), None)?;

            let data = b"persistent index data".to_vec();
            let hash = Hash::from_blake3(blake3::hash(&data));
            store.put(&hash, &data)?;
            store.flush()?;
        }

        // Reopen with fresh index — should populate via _meta.index
        let index2 = Arc::new(HashIndex::new(1_000_000));
        let store2 = CloudStore::open(op, index2.clone(), None)?;

        let data = b"persistent index data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        assert!(store2.contains(&hash)?);
        let retrieved = store2.get(&hash)?;
        assert_eq!(retrieved, data);
        Ok(())
    }

    #[test]
    fn test_cloud_store_cache_hit() -> Result<()> {
        let tmp = tempfile::TempDir::new().unwrap();
        let op = Operator::new(services::Memory::default())
            .map_err(|e| PacktError::Cloud {
                context: "failed to create memory operator".into(),
                source: e,
            })?
            .finish();
        let index = Arc::new(HashIndex::new(1_000_000));
        let store = CloudStore::open(op.clone(), index.clone(), Some(tmp.path().to_path_buf()))?;

        let data = b"cached test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;

        // First get: downloads from cloud (no cache hit)
        let result = store.get(&hash)?;
        assert_eq!(result, data);

        // Second get: should hit cache
        let result2 = store.get(&hash)?;
        assert_eq!(result2, data);

        Ok(())
    }

    #[test]
    fn test_cloud_store_cache_disabled() -> Result<()> {
        let op = Operator::new(services::Memory::default())
            .map_err(|e| PacktError::Cloud {
                context: "failed to create memory operator".into(),
                source: e,
            })?
            .finish();
        let index = Arc::new(HashIndex::new(1_000_000));
        // No cache_dir = cache disabled
        let store = CloudStore::open(op, index.clone(), None)?;

        let data = b"no cache data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;

        let result = store.get(&hash)?;
        assert_eq!(result, data);
        Ok(())
    }

    #[test]
    fn test_cloud_store_cache_persists_across_opens() -> Result<()> {
        let tmp = tempfile::TempDir::new().unwrap();
        let op = Operator::new(services::Memory::default())
            .map_err(|e| PacktError::Cloud {
                context: "failed to create memory operator".into(),
                source: e,
            })?
            .finish();

        // First session with cache
        let index = Arc::new(HashIndex::new(1_000_000));
        let store = CloudStore::open(op.clone(), index.clone(), Some(tmp.path().to_path_buf()))?;
        let data = b"cache persist data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data)?;
        store.flush()?;

        // Read to populate cache
        store.get(&hash)?;
        drop(store);

        // Second session with same cache dir — cache file exists
        // but index must be repopulated from _meta.index
        let index2 = Arc::new(HashIndex::new(1_000_000));
        let store2 = CloudStore::open(op, index2.clone(), Some(tmp.path().to_path_buf()))?;
        let result = store2.get(&hash)?;
        assert_eq!(result, data);
        Ok(())
    }
}
