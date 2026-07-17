use crate::error::{PacktError, Result};
use crate::index::DedupIndex;
use crate::store::ContentStore;
use crate::store::pack;
use crate::store::pack::IndexEntry;
use crate::types::{Hash, PackLocation};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Local filesystem content-addressed store.
///
/// Layout:
///   {root}/packs/{pack_id}.pack  — Pack files containing compressed chunks
pub struct LocalStore {
    root: PathBuf,
    state: Mutex<StoreState>,
    pack_target_size: u64,
    next_pack_id: AtomicU32,
    index: Mutex<Option<Arc<dyn DedupIndex>>>,
}

struct StoreState {
    /// In-memory buffer for the current pending pack
    pending_chunks: Vec<PendingEntry>,
    /// Size of pending data so far
    pending_size: u64,
    /// On-disk pack files loaded
    packs: HashMap<u32, PackMetadata>,
}

struct PendingEntry {
    hash: Hash,
    data: Vec<u8>,
    orig_length: u32,
    entry_type: pack::EntryType,
    signature: Option<Vec<u8>>,
}

#[derive(Default)]
struct PackMetadata {
    entries: Vec<IndexEntry>,
    superblock: Option<Vec<u8>>,
}

impl LocalStore {
    /// Create or open a local store at `root`.
    ///
    /// # Errors
    /// Returns error if the root directory cannot be created/read.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let packs_dir = root.join("packs");
        std::fs::create_dir_all(&packs_dir)?;

        // Load existing pack files
        let mut packs = HashMap::new();
        let mut next_pack_id = 0u32;

        let mut pack_files: Vec<_> = std::fs::read_dir(&packs_dir)?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "pack"))
            .collect();
        pack_files.sort_by_key(std::fs::DirEntry::file_name);

        for entry in &pack_files {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(stem) = name_str.strip_suffix(".pack")
                && let Ok(id) = stem.parse::<u32>()
            {
                let path = entry.path();
                let data = std::fs::read(&path)?;
                match pack::read_pack(&data) {
                    Ok((entries, _checksum, superblock)) => {
                        packs.insert(id, PackMetadata { entries, superblock });
                        next_pack_id = id + 1;
                    }
                    Err(e) => {
                        return Err(PacktError::StoreCorrupted(format!(
                            "Pack file {} is corrupted: {e}",
                            path.display()
                        )));
                    }
                }
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            state: Mutex::new(StoreState {
                pending_chunks: Vec::new(),
                pending_size: 0,
                packs,
            }),
            pack_target_size: 16 * 1024 * 1024, // 16 MB default
            next_pack_id: AtomicU32::new(next_pack_id),
            index: Mutex::new(None),
        })
    }

    /// Phase 1: drain pending chunks under the lock, return data for I/O.
    fn flush_prepare(state: &mut StoreState, _pack_id: u32) -> Vec<pack::PackEntry> {
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

    /// Phase 2: write pack to disk (no lock held).
    fn flush_write(chunks: &[pack::PackEntry], pack_id: u32, root: &Path) -> Result<PackMetadata> {
        if chunks.is_empty() {
            return Ok(PackMetadata::default());
        }

        let pack_bytes = pack::write_pack(chunks)?;
        let packs_dir = root.join("packs");
        let tmp_path = packs_dir.join(format!("{pack_id}.tmp"));
        let final_path = packs_dir.join(format!("{pack_id}.pack"));

        std::fs::write(&tmp_path, &pack_bytes)?;
        let file = std::fs::File::open(&tmp_path)?;
        file.sync_all()?;
        drop(file);
        let mut retries = 0;
        loop {
            match std::fs::rename(&tmp_path, &final_path) {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && retries < 10 => {
                    retries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50 * retries));
                }
                Err(e) => return Err(e.into()),
            }
        }
        if let Ok(dir) = std::fs::File::open(&packs_dir) {
            dir.sync_all()?;
        }

        // Parse the pack metadata from what we just wrote.
        // (read_pack verifies checksum; if this fails the write is corrupt.)
        match pack::read_pack(&pack_bytes) {
            Ok((entries, _checksum, superblock)) => Ok(PackMetadata { entries, superblock }),
            Err(e) => Err(PacktError::StoreCorrupted(format!(
                "Newly-constructed pack {} failed internal verification: {e}",
                final_path.display()
            ))),
        }
    }

    pub fn populate_index(&self, index: &Arc<dyn DedupIndex>) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
        for (&pack_id, pack) in &state.packs {
            for entry in &pack.entries {
                let loc = PackLocation {
                    pack_id,
                    offset: entry.offset,
                    length: entry.length,
                    orig_length: entry.orig_length,
                };
                index.insert(entry.hash, loc);
            }
        }
        Ok(())
    }

    pub fn set_index(&self, index: Arc<dyn DedupIndex>) {
        if let Ok(mut guard) = self.index.lock() {
            *guard = Some(index);
        }
    }

    /// Rebuild a PalantirIndex from stored chunk signatures.
    /// Iterates all pack entries with signatures and inserts them into the index.
    pub fn rebuild_similarity_index(&self, index: &mut crate::similarity::palantir::PalantirIndex) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
        let mut entries = Vec::new();
        for pack in state.packs.values() {
            for entry in &pack.entries {
                if let Some(sig_bytes) = &entry.signature {
                    if let Ok((sig, _)) =
                        postcard::take_from_bytes::<crate::similarity::super_feature::ChunkSignature>(sig_bytes)
                    {
                        entries.push((entry.hash, sig));
                    }
                }
            }
        }
        index.rebuild(entries);
        Ok(())
    }

    fn get_inner(&self, hash: &Hash, depth: usize) -> Result<Vec<u8>> {
        if depth > 16 {
            return Err(PacktError::InvalidPackFormat(
                "delta chain too deep (possible cycle)".into(),
            ));
        }

        // Phase 1: Check pending chunks first (always under lock, but pending is small).
        {
            let state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
            for entry in &state.pending_chunks {
                if entry.hash == *hash {
                    return Ok(entry.data.clone());
                }
            }
        }

        // Phase 2: Try the dedup index for O(1) lookup.
        // IMPORTANT: Guard must be dropped before read_from_location because
        // delta chunks recursively call get_inner, which would re-lock index
        // and deadlock (std Mutex is not reentrant).
        let loc = if let Ok(guard) = self.index.lock() {
            if let Some(ref index) = *guard {
                index.lookup(hash)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(loc) = loc {
            return self.read_from_location(hash, loc, depth);
        }

        // Phase 3: Fall back to linear scan (for index misses / missing index).
        let match_info = {
            let state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;

            let mut info = None;
            for (pack_id, pack) in &state.packs {
                for entry in &pack.entries {
                    if &entry.hash == hash {
                        let pack_path = self.root.join("packs").join(format!("{pack_id}.pack"));
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
                        info = Some((pack_path, loc, base_hash, needs_raw, pack.superblock.clone()));
                        break;
                    }
                }
                if info.is_some() {
                    break;
                }
            }
            info
        };

        let Some((pack_path, loc, base_hash, needs_raw, superblock)) = match_info else {
            return Err(PacktError::ChunkNotFound(hash.to_hex()));
        };

        let pack_data = std::fs::read(&pack_path)?;
        self.read_chunk_data(
            hash,
            &pack_data,
            &loc,
            base_hash,
            needs_raw,
            superblock.as_deref(),
            depth,
        )
    }

    /// Fast path: read chunk directly by `PackLocation` from index lookup.
    fn read_from_location(&self, hash: &Hash, loc: PackLocation, depth: usize) -> Result<Vec<u8>> {
        let pack_path = self.root.join("packs").join(format!("{}.pack", loc.pack_id));

        // Get the entry metadata for this location from the pack state.
        let (base_hash, needs_raw, superblock) = {
            let state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
            let Some(pack) = state.packs.get(&loc.pack_id) else {
                return Err(PacktError::ChunkNotFound(hash.to_hex()));
            };
            // Find the matching entry by offset (PackLocation offset is unique per pack).
            let mut found = None;
            for entry in &pack.entries {
                if entry.offset == loc.offset && entry.length == loc.length {
                    let (base_hash, needs_raw) = match &entry.entry_type {
                        pack::EntryType::Delta { base_hash } => (Some(*base_hash), false),
                        pack::EntryType::Full => (None, false),
                        pack::EntryType::FullRaw => (None, true),
                    };
                    found = Some((base_hash, needs_raw, pack.superblock.clone()));
                    break;
                }
            }
            found.ok_or_else(|| PacktError::ChunkNotFound(hash.to_hex()))?
        };

        let pack_data = std::fs::read(&pack_path)?;
        self.read_chunk_data(
            hash,
            &pack_data,
            &loc,
            base_hash,
            needs_raw,
            superblock.as_deref(),
            depth,
        )
    }

    /// O(1) lookup via DedupIndex. Returns None if index is not set or miss.
    fn lookup_index(&self, hash: &Hash) -> Option<PackLocation> {
        if let Ok(guard) = self.index.lock() {
            if let Some(ref idx) = *guard {
                return idx.lookup(hash);
            }
        }
        None
    }

    /// Shared chunk data reader: decompress + verify checksum.
    #[allow(clippy::too_many_arguments)]
    fn read_chunk_data(
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

impl ContentStore for LocalStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation> {
        // Phase 1: DedupIndex O(1) check (replaces O(n) linear pack scan).
        if let Some(loc) = self.lookup_index(hash) {
            return Ok(loc);
        }

        // Phase 2: Check pending chunks (small set, must check under lock).
        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
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

        // Phase 3: Append to pending buffer.
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
            let chunks = Self::flush_prepare(&mut state, pack_id);
            let root = self.root.clone();
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &root)?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
        }

        Ok(PackLocation {
            pack_id: self.next_pack_id.load(Ordering::SeqCst),
            offset: 0,
            length: 0,
            orig_length,
        })
    }

    fn put_delta(&self, hash: &Hash, base_hash: &Hash, delta_data: &[u8], orig_length: u32) -> Result<PackLocation> {
        // Phase 1: DedupIndex O(1) check (replaces O(n) linear pack scan).
        if let Some(loc) = self.lookup_index(hash) {
            return Ok(loc);
        }

        // Phase 2: Check pending chunks (small set, must check under lock).
        let mut state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
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

        // Phase 3: Append to pending buffer.
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
            let chunks = Self::flush_prepare(&mut state, pack_id);
            let root = self.root.clone();
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &root)?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
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
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
        for entry in &mut state.pending_chunks {
            if entry.hash == *hash {
                entry.signature = Some(signature.to_vec());
                return Ok(());
            }
        }
        Err(PacktError::ChunkNotFound(hash.to_hex()))
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        LocalStore::get_inner(self, hash, 0)
    }

    fn contains(&self, hash: &Hash) -> Result<bool> {
        // O(1) check via DedupIndex.
        if self.lookup_index(hash).is_some() {
            return Ok(true);
        }
        // Fall back to linear scan of pending + packs for safety
        // (index may not be populated in all code paths).
        let state = self
            .state
            .lock()
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(true);
            }
        }
        for pack in state.packs.values() {
            for entry in &pack.entries {
                if &entry.hash == hash {
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
            .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
        if !state.pending_chunks.is_empty() {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            let chunks = Self::flush_prepare(&mut state, pack_id);
            let root = self.root.clone();
            drop(state);
            let meta = Self::flush_write(&chunks, pack_id, &root)?;
            let mut state = self
                .state
                .lock()
                .map_err(|e| PacktError::StoreCorrupted(format!("store lock poisoned: {e}")))?;
            state.packs.insert(pack_id, meta);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ContentStore;
    use crate::types::Hash;
    use tempfile::TempDir;

    fn setup_store() -> (TempDir, LocalStore) {
        let dir = TempDir::new().unwrap();
        let store = LocalStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn test_store_roundtrip() {
        let (_dir, store) = setup_store();
        let data = b"hello world this is a test chunk".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let _loc = store.put(&hash, &data).unwrap();
        // Flush so data is on disk
        store.flush().unwrap();
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_store_dedup() {
        let (_dir, store) = setup_store();
        let data = b"dedup test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));

        store.put(&hash, &data).unwrap();
        store.flush().unwrap();

        // Put same data again — should succeed without error
        store.put(&hash, &data).unwrap();
        store.flush().unwrap();
    }

    #[test]
    fn test_store_missing() {
        let (_dir, store) = setup_store();
        let hash = Hash::from_bytes([0u8; 32]);
        let result = store.get(&hash);
        assert!(result.is_err(), "Getting nonexistent chunk should fail");
    }

    #[test]
    fn test_store_contains() {
        let (_dir, store) = setup_store();
        let data = b"test contains".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));

        assert!(!store.contains(&hash).unwrap());
        store.put(&hash, &data).unwrap();
        store.flush().unwrap();
        assert!(store.contains(&hash).unwrap());
    }

    #[test]
    fn test_store_large_chunk() {
        let (_dir, store) = setup_store();
        let data = vec![0xABu8; 1_000_000]; // 1 MB chunk
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data).unwrap();
        store.flush().unwrap();
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved.len(), data.len());
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_store_integrity_check() {
        let (_dir, store) = setup_store();
        let data = b"integrity check data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        store.put(&hash, &data).unwrap();
        store.flush().unwrap();

        // Retrieve and verify
        let retrieved = store.get(&hash).unwrap();
        let verify = blake3::hash(&retrieved);
        assert_eq!(Hash::from_blake3(verify), hash);
    }

    #[test]
    fn test_store_delta_roundtrip() {
        let (_dir, store) = setup_store();
        let base_data: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let base_hash = Hash::from_blake3(blake3::hash(&base_data));
        store.put(&base_hash, &base_data).unwrap();
        store.flush().unwrap();

        let mut target_data = base_data.clone();
        target_data[100] = 0xFF;
        let target_hash = Hash::from_blake3(blake3::hash(&target_data));

        let encoder = crate::store::delta::DeltaEncoder::new(3);
        let delta = encoder
            .try_encode(&base_data, &target_data)
            .unwrap()
            .expect("delta should be beneficial for similar data");

        let _loc = store
            .put_delta(&target_hash, &base_hash, &delta, target_data.len() as u32)
            .unwrap();
        store.flush().unwrap();

        let retrieved = store.get(&target_hash).unwrap();
        assert_eq!(retrieved, target_data, "delta roundtrip should reconstruct original");
    }

    #[test]
    fn test_store_delta_base_not_found() {
        let (_dir, store) = setup_store();
        let base_data: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
        let mut data = base_data.clone();
        data[100] = 0xFF; // small change like roundtrip test
        let hash = Hash::from_blake3(blake3::hash(&data));
        let fake_base = Hash::from_bytes([0u8; 32]);

        let encoder = crate::store::delta::DeltaEncoder::new(3);
        let delta = encoder
            .try_encode(&base_data, &data)
            .unwrap()
            .expect("delta should work");

        let _loc = store.put_delta(&hash, &fake_base, &delta, data.len() as u32).unwrap();
        store.flush().unwrap();

        let result = store.get(&hash);
        assert!(result.is_err(), "should fail when base chunk is missing");
    }

    #[test]
    fn test_store_multiple_chunks() {
        let (_dir, store) = setup_store();
        let chunks: Vec<_> = (0..100)
            .map(|i| {
                let data = format!("chunk number {i} with some data to make it unique");
                let hash = Hash::from_blake3(blake3::hash(data.as_bytes()));
                (hash, data.into_bytes())
            })
            .collect();

        for (hash, data) in &chunks {
            store.put(hash, data).unwrap();
        }
        store.flush().unwrap();

        for (hash, data) in &chunks {
            let retrieved = store.get(hash).unwrap();
            assert_eq!(&retrieved, data);
        }
    }
}
