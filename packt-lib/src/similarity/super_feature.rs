/// A 3-tier hierarchical super-feature signature for a data chunk.
///
/// Feature extraction: divide chunk into 12 fixed sub-chunks, hash first 32
/// bytes of each with xxh3. Position-stable — modifications in a sub-chunk
/// only affect that sub-chunk's feature if they hit the first 32 bytes.
///
/// LIMITATION: Only the first 32 bytes of each sub-chunk are sampled.
/// Modifications at offset >=32 within a sub-chunk are invisible to
/// similarity detection. This affects ~1.2% of each 2730-byte sub-chunk
/// at default 32KB chunk size. Acceptable because (1) modifications are
/// localized in real workloads, (2) the head+tail FPR filter provides a
/// second check, and (3) more comprehensive sampling would increase
/// feature extraction cost. For chunks < 256 bytes, falls back to
/// full-chunk hash.
///
/// Group 12 features into 3 tiers (Palantir-style hierarchical matching):
///   Tier 1 (>=95%): 3 SFs x 4 features
///   Tier 2 (>=85%): 4 SFs x 3 features
///   Tier 3 (>=70%): 6 SFs x 2 features
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkSignature {
    pub tier1: [u64; 3],
    pub tier2: [u64; 4],
    pub tier3: [u64; 6],
}

const NUM_FEATURES: usize = 12;
const SAMPLE_BYTES: usize = 32;
const SMALL_CHUNK_THRESHOLD: usize = 256;

use serde::{Deserialize, Serialize};

pub fn extract_signature(data: &[u8]) -> Option<ChunkSignature> {
    if data.len() < 64 {
        return None;
    }
    if data.len() < SMALL_CHUNK_THRESHOLD {
        // Small chunk: use full-chunk hash for all tiers
        // 12 sub-chunks would each sample only ~5 bytes — too fragile
        return Some(full_chunk_signature(data));
    }
    Some(group_super_features(&compute_features(data)))
}

fn full_chunk_signature(data: &[u8]) -> ChunkSignature {
    let h = xxhash_rust::xxh3::xxh3_64(data);
    ChunkSignature {
        tier1: [h, h, h],
        tier2: [h, h, h, h],
        tier3: [h, h, h, h, h, h],
    }
}

fn compute_features(data: &[u8]) -> [u64; NUM_FEATURES] {
    let mut features = [0u64; NUM_FEATURES];
    let sc_size = data.len() / NUM_FEATURES;
    for (i, f) in features.iter_mut().enumerate() {
        let start = i * sc_size;
        let end = if i == NUM_FEATURES - 1 {
            data.len()
        } else {
            start + sc_size
        };
        let sample = &data[start..end][..SAMPLE_BYTES.min(sc_size)];
        *f = xxhash_rust::xxh3::xxh3_64(sample);
    }
    features
}

fn group_super_features(features: &[u64; NUM_FEATURES]) -> ChunkSignature {
    #[allow(clippy::unreadable_literal)]
    fn sf_h(f: &[u64; NUM_FEATURES], s: usize, c: usize) -> u64 {
        let mut h = 0u64;
        for &v in f.iter().skip(s).take(c) {
            h = h.wrapping_mul(0x9e3779b97f4a7c15) ^ v;
        }
        h
    }
    ChunkSignature {
        tier1: [sf_h(features, 0, 4), sf_h(features, 4, 4), sf_h(features, 8, 4)],
        tier2: [
            sf_h(features, 0, 3),
            sf_h(features, 3, 3),
            sf_h(features, 6, 3),
            sf_h(features, 9, 3),
        ],
        tier3: [
            sf_h(features, 0, 2),
            sf_h(features, 2, 2),
            sf_h(features, 4, 2),
            sf_h(features, 6, 2),
            sf_h(features, 8, 2),
            sf_h(features, 10, 2),
        ],
    }
}

pub fn check_similarity(a: &[u8], b: &[u8]) -> bool {
    use xxhash_rust::xxh3::xxh3_64;
    let head_a = &a[..64.min(a.len())];
    let head_b = &b[..64.min(b.len())];
    if xxh3_64(head_a) == xxh3_64(head_b) {
        return true;
    }
    // Only compare tail if chunks are large enough that tail doesn't overlap head
    if a.len() > 64 && b.len() > 64 {
        let tail_a = &a[a.len().saturating_sub(64)..];
        let tail_b = &b[b.len().saturating_sub(64)..];
        // Avoid comparing the same bytes twice for chunks 65-128 bytes
        // where head and tail windows overlap
        if tail_a.as_ptr() != head_a.as_ptr() || a.len() != b.len() {
            return xxh3_64(tail_a) == xxh3_64(tail_b);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mk_sig(data: &[u8]) -> ChunkSignature {
        extract_signature(data).unwrap()
    }

    #[test]
    fn test_identical() {
        let d = vec![42u8; 4096];
        assert_eq!(mk_sig(&d), mk_sig(&d));
    }
    #[test]
    fn test_small_none() {
        assert!(extract_signature(b"too small").is_none());
    }
    #[test]
    fn test_small_chunk_fallback() {
        // 128-byte chunk should use full-hash fallback (not sub-chunks)
        let d = vec![0xABu8; 128];
        let sig = mk_sig(&d);
        // All tiers should have the same hash (full-chunk hash)
        assert_eq!(sig.tier1[0], sig.tier2[0]);
        assert_eq!(sig.tier2[0], sig.tier3[0]);
        // 64-byte chunk should work (borderline)
        let d64 = vec![0xABu8; 64];
        let sig64 = mk_sig(&d64);
        assert!(sig64.tier1.iter().any(|&sf| sf != 0));
    }

    #[test]
    fn test_similar_share_sf() {
        let base: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let sb = mk_sig(&base);
        let mut m = base.clone();
        for i in 0..80 {
            m[i * 50] = 0xFF;
        }
        let sm = mk_sig(&m);
        let t1 = sb.tier1.iter().zip(sm.tier1.iter()).filter(|(a, b)| a == b).count();
        let t2 = sb.tier2.iter().zip(sm.tier2.iter()).filter(|(a, b)| a == b).count();
        let t3 = sb.tier3.iter().zip(sm.tier3.iter()).filter(|(a, b)| a == b).count();
        assert!(t1 + t2 + t3 > 0, "no matching SF (t1={t1} t2={t2} t3={t3})");
    }

    #[test]
    fn test_different() {
        assert_ne!(mk_sig(&[0xABu8; 4096]), mk_sig(&[0xCDu8; 4096]));
    }
    #[test]
    fn test_fpr_identical() {
        assert!(check_similarity(&[0u8; 256], &[0u8; 256]));
    }
    #[test]
    fn test_fpr_different() {
        assert!(!check_similarity(&[0xABu8; 256], &[0xCDu8; 256]));
    }
    #[test]
    fn test_fpr_head_preserved() {
        let b: Vec<u8> = (0..256).map(|i| (i % 251) as u8).collect();
        let mut m = b.clone();
        m[100..200].fill(0xFF);
        assert!(check_similarity(&b, &m));
    }
}
