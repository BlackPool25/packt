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
}

#[derive(Default)]
struct PackMetadata {
    entries: Vec<IndexEntry>,
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
                    Ok((entries, _checksum)) => {
                        packs.insert(id, PackMetadata { entries });
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
        })
    }

    fn flush_pack(state: &mut StoreState, pack_id: u32, root: &Path) -> Result<()> {
        let chunks: Vec<_> = state
            .pending_chunks
            .drain(..)
            .map(|e| (e.hash, e.data, e.orig_length))
            .collect();

        if chunks.is_empty() {
            return Ok(());
        }

        let pack_bytes = pack::write_pack(&chunks)?;
        let packs_dir = root.join("packs");
        let tmp_path = packs_dir.join(format!("{pack_id}.tmp"));
        let final_path = packs_dir.join(format!("{pack_id}.pack"));

        std::fs::write(&tmp_path, &pack_bytes)?;
        let file = std::fs::File::open(&tmp_path)?;
        file.sync_all()?;
        drop(file);
        // Atomic rename
        std::fs::rename(&tmp_path, &final_path)?;
        // fsync the directory
        if let Ok(dir) = std::fs::File::open(&packs_dir) {
            dir.sync_all()?;
        }

        // Re-read pack to get index
        let data = std::fs::read(&final_path)?;
        match pack::read_pack(&data) {
            Ok((entries, _checksum)) => {
                state.packs.insert(pack_id, PackMetadata { entries });
            }
            Err(e) => {
                return Err(PacktError::StoreCorrupted(format!(
                    "Just-written pack {} failed verification: {e}",
                    final_path.display()
                )));
            }
        }

        state.pending_size = 0;
        Ok(())
    }

    pub fn populate_index(&self, index: &Arc<dyn DedupIndex>) -> Result<()> {
        let state = self.state.lock().unwrap();
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
}

impl ContentStore for LocalStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation> {
        let mut state = self.state.lock().unwrap();

        for pack in state.packs.values() {
            for entry in &pack.entries {
                if &entry.hash == hash {
                    return Ok(PackLocation {
                        pack_id: 0,
                        offset: entry.offset,
                        length: entry.length,
                        orig_length: entry.orig_length,
                    });
                }
            }
        }
        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(PackLocation {
                    pack_id: self.next_pack_id.load(Ordering::Relaxed),
                    offset: 0,
                    length: 0,
                    orig_length: entry.orig_length,
                });
            }
        }

        let orig_length = data.len() as u32;
        state.pending_chunks.push(PendingEntry {
            hash: *hash,
            data: data.to_vec(),
            orig_length,
        });
        state.pending_size += data.len() as u64;

        if state.pending_size >= self.pack_target_size {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            Self::flush_pack(&mut state, pack_id, &self.root)?;
        }

        Ok(PackLocation {
            pack_id: self.next_pack_id.load(Ordering::Relaxed),
            offset: 0,
            length: 0,
            orig_length,
        })
    }

    fn get(&self, hash: &Hash) -> Result<Vec<u8>> {
        let state = self.state.lock().unwrap();

        for entry in &state.pending_chunks {
            if entry.hash == *hash {
                return Ok(entry.data.clone());
            }
        }

        for (pack_id, pack) in &state.packs {
            for entry in &pack.entries {
                if &entry.hash == hash {
                    let pack_path = self.root.join("packs").join(format!("{pack_id}.pack"));
                    let data = std::fs::read(&pack_path)?;
                    let loc = PackLocation {
                        pack_id: *pack_id,
                        offset: entry.offset,
                        length: entry.length,
                        orig_length: entry.orig_length,
                    };
                    let stored_data = pack::read_chunk(&data, &loc)?;
                    let actual_hash = blake3::hash(&stored_data);
                    if Hash::from_blake3(actual_hash) != *hash {
                        return Err(PacktError::ChecksumMismatch {
                            expected: hash.to_hex(),
                            actual: Hash::from_blake3(actual_hash).to_hex(),
                        });
                    }
                    return Ok(stored_data);
                }
            }
        }

        Err(PacktError::ChunkNotFound(hash.to_hex()))
    }

    fn contains(&self, hash: &Hash) -> Result<bool> {
        let state = self.state.lock().unwrap();

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
        let mut state = self.state.lock().unwrap();
        if !state.pending_chunks.is_empty() {
            let pack_id = self.next_pack_id.fetch_add(1, Ordering::SeqCst);
            Self::flush_pack(&mut state, pack_id, &self.root)?;
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
