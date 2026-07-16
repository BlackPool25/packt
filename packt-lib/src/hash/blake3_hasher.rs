use crate::hash::ContentHasher;
use crate::types::{Chunk, Hash};

/// BLAKE3 content hasher.
pub struct Blake3Hasher;

impl Blake3Hasher {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Blake3Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentHasher for Blake3Hasher {
    fn hash(&self, data: &[u8]) -> Hash {
        let hash = blake3::hash(data);
        Hash::from_blake3(hash)
    }

    fn hash_chunk(&self, chunk: &Chunk) -> Hash {
        self.hash(&chunk.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::ContentHasher;
    use proptest::prelude::*;

    #[test]
    fn test_blake3_empty() {
        let hasher = Blake3Hasher::new();
        let hash = hasher.hash(b"");
        // Known blake3 hash of empty string
        assert_eq!(
            hash.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn test_blake3_known_values() {
        let hasher = Blake3Hasher::new();
        let hash = hasher.hash(b"abc");
        assert_eq!(
            hash.to_hex(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn test_blake3_determinism() {
        let hasher = Blake3Hasher::new();
        let data = b"the quick brown fox jumps over the lazy dog";
        let h1 = hasher.hash(data);
        let h2 = hasher.hash(data);
        assert_eq!(h1, h2, "Same input must produce same hash");
    }

    #[test]
    fn test_blake3_chunk() {
        let hasher = Blake3Hasher::new();
        let chunk = Chunk {
            offset: 0,
            length: 3,
            data: b"abc".to_vec(),
        };
        let hash = hasher.hash_chunk(&chunk);
        assert_eq!(
            hash.to_hex(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn test_hash_to_hex_roundtrip() {
        let hasher = Blake3Hasher::new();
        let original = hasher.hash(b"roundtrip test data");
        let hex = original.to_hex();
        let parsed = Hash::from_hex(&hex).expect("Valid hex should parse");
        assert_eq!(original, parsed);
    }

    proptest! {
        #[test]
        fn test_blake3_collision_resistance(data1: Vec<u8>, data2: Vec<u8>) {
            let hasher = Blake3Hasher::new();
            let h1 = hasher.hash(&data1);
            let h2 = hasher.hash(&data2);
            if data1 != data2 {
                prop_assert_ne!(h1, h2,
                    "Different inputs should produce different hashes (birthday paradox negligible)");
            }
        }
    }

    proptest! {
        #[test]
        fn test_blake3_streaming_matches_oneshot(chunks: Vec<Vec<u8>>) {
            let hasher = Blake3Hasher::new();

            // One-shot hash of all concatenated data
            let mut all_data = Vec::new();
            for c in &chunks {
                all_data.extend_from_slice(c);
            }
            let oneshot = hasher.hash(&all_data);

            // Streaming hash via blake3's incrementala API
            let mut incremental = blake3::Hasher::new();
            for c in &chunks {
                incremental.update(c);
            }
            let streamed = Hash::from_blake3(incremental.finalize());

            prop_assert_eq!(oneshot, streamed,
                "Streaming hash must match one-shot hash");
        }
    }
}
