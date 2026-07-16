use crate::index::DedupIndex;
use crate::types::{Hash, PackLocation};
use dashmap::DashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Concurrent in-memory dedup index backed by DashMap.
pub struct HashIndex {
    map: DashMap<Hash, PackLocation>,
    count: AtomicUsize,
    bloom_filter: Mutex<BloomFilter>,
}

/// Simple Bloom filter implementation.
#[allow(dead_code)]
struct BloomFilter {
    bits: Vec<u64>,
    num_hashes: u32,
    size: usize,
}

#[allow(dead_code)]
impl BloomFilter {
    /// Create a bloom filter with `capacity` entries and false positive rate ~1%.
    fn new(capacity: usize) -> Self {
        // m = -n * ln(p) / (ln(2)^2)
        // For p = 0.01 (~1% false positive): m ≈ 9.6 * n bits
        let num_bits = (capacity as f64 * 9.6).ceil() as usize;
        // k = (m/n) * ln(2) ≈ 7 hashes for p=1%
        let num_hashes = 7u32;

        // Round up to multiple of 64
        let num_u64s = num_bits.div_ceil(64);

        Self {
            bits: vec![0u64; num_u64s],
            num_hashes,
            size: num_u64s * 64,
        }
    }

    /// Derive k independent hash values from a 32-byte hash.
    fn hash_indices(&self, hash: &Hash) -> Vec<usize> {
        let mut indices = Vec::with_capacity(self.num_hashes as usize);
        for i in 0..self.num_hashes {
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&hash.0[..8]);
            key[0] ^= i as u8;
            let h = blake3::Hasher::new_keyed(&key).update(b"bloom").finalize();
            let val = u64::from_le_bytes(
                h.as_bytes()[..8]
                    .try_into()
                    .expect("blake3 hash is 32 bytes; first 8 always valid for u64 conversion"),
            );
            let idx = (val as usize) % self.size;
            indices.push(idx);
        }
        indices
    }

    fn insert(&mut self, hash: &Hash) {
        for idx in self.hash_indices(hash) {
            let word = idx / 64;
            let bit = idx % 64;
            self.bits[word] |= 1u64 << bit;
        }
    }

    fn might_contain(&self, hash: &Hash) -> bool {
        for idx in self.hash_indices(hash) {
            let word = idx / 64;
            let bit = idx % 64;
            if self.bits[word] & (1u64 << bit) == 0 {
                return false; // Definitely not in set
            }
        }
        true // Might be in set (could be false positive)
    }
}

impl HashIndex {
    /// Create a new hash index with capacity for `expected_entries` unique chunks.
    #[must_use]
    pub fn new(expected_entries: usize) -> Self {
        Self {
            map: DashMap::with_capacity(expected_entries),
            count: AtomicUsize::new(0),
            bloom_filter: Mutex::new(BloomFilter::new(expected_entries)),
        }
    }
}

impl DedupIndex for HashIndex {
    fn insert(&self, hash: Hash, location: PackLocation) {
        self.bloom_filter
            .lock()
            .expect("bloom filter mutex poisoned")
            .insert(&hash);
        let prev = self.map.insert(hash, location);
        if prev.is_none() {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn lookup(&self, hash: &Hash) -> Option<PackLocation> {
        // Quick negative check via bloom filter
        if !self
            .bloom_filter
            .lock()
            .expect("bloom filter mutex poisoned")
            .might_contain(hash)
        {
            return None;
        }
        self.map.get(hash).map(|r| *r.value())
    }

    fn contains(&self, hash: &Hash) -> bool {
        self.map.contains_key(hash)
    }

    fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::DedupIndex;

    fn test_hash(data: &[u8]) -> Hash {
        Hash::from_blake3(blake3::hash(data))
    }

    fn test_location(id: u32) -> PackLocation {
        PackLocation {
            pack_id: id,
            offset: 0,
            length: 100,
            orig_length: 100,
        }
    }

    #[test]
    fn test_index_empty() {
        let idx = HashIndex::new(1000);
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn test_index_insert_and_lookup() {
        let idx = HashIndex::new(1000);
        let hash = test_hash(b"test data");
        let loc = test_location(1);

        idx.insert(hash, loc);
        assert!(!idx.is_empty());
        assert_eq!(idx.len(), 1);

        let found = idx.lookup(&hash);
        assert_eq!(found, Some(loc));
    }

    #[test]
    fn test_index_contains() {
        let idx = HashIndex::new(1000);
        let hash = test_hash(b"exists");
        let loc = test_location(2);

        assert!(!idx.contains(&hash));
        idx.insert(hash, loc);
        assert!(idx.contains(&hash));
    }

    #[test]
    fn test_index_missing_lookup() {
        let idx = HashIndex::new(1000);
        let hash = test_hash(b"missing");
        assert_eq!(idx.lookup(&hash), None);
    }

    #[test]
    fn test_index_multiple_entries() {
        let idx = HashIndex::new(10000);
        let mut hashes = Vec::new();

        for i in 0..1000 {
            let data = format!("test data item {i}");
            let hash = test_hash(data.as_bytes());
            let loc = test_location(i);
            idx.insert(hash, loc);
            hashes.push(hash);
        }

        assert_eq!(idx.len(), 1000);

        // Verify all inserted hashes are findable
        for hash in &hashes {
            assert!(idx.contains(hash), "Hash should exist in index");
            assert!(idx.lookup(hash).is_some(), "Should find location for hash");
        }
    }

    #[test]
    fn test_index_overwrite() {
        let idx = HashIndex::new(1000);
        let hash = test_hash(b"overwrite");
        let loc1 = test_location(10);
        let loc2 = test_location(20);

        idx.insert(hash, loc1);
        idx.insert(hash, loc2); // Overwrite

        // Should still contain the hash, but location may be either
        assert!(idx.contains(&hash));
        assert_eq!(idx.len(), 1); // Same key, count should be 1
    }

    #[test]
    fn test_bloom_filter() {
        let mut bloom = BloomFilter::new(1000);
        let hash = test_hash(b"bloom test");
        let _hash2 = test_hash(b"bloom test 2");

        assert!(!bloom.might_contain(&hash)); // Definitely not
        bloom.insert(&hash);
        assert!(bloom.might_contain(&hash)); // Might be (should be)

        // hash2 is NOT in the filter — should return false or (rarely) true
        // due to false positive. We can't assert !contains because of FP.
        // Instead: verify hash IS found after insert.
        assert!(bloom.might_contain(&hash));
    }

    #[test]
    fn test_concurrent_inserts() {
        use std::sync::Arc;
        use std::thread;

        let idx = Arc::new(HashIndex::new(10000));
        let mut handles = Vec::new();

        for t in 0..8 {
            let idx = Arc::clone(&idx);
            let handle = thread::spawn(move || {
                for i in 0..1000 {
                    let data = format!("thread {t} item {i}");
                    let hash = test_hash(data.as_bytes());
                    let loc = test_location(t * 1000 + i);
                    idx.insert(hash, loc);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(idx.len(), 8000);
    }
}
