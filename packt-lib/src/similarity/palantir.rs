use crate::similarity::super_feature::ChunkSignature;
use crate::types::Hash;
use std::collections::{HashMap, VecDeque};

/// Tier of similarity detection (from Palantir hierarchical matching).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SimilarityTier {
    High = 3,
    Medium = 2,
    Low = 1,
}

/// A candidate match from the Palantir index.
#[derive(Debug, Clone)]
pub struct MatchCandidate {
    pub hash: Hash,
    pub tier: SimilarityTier,
}

/// Hierarchical super-feature index (Palantir-style).
pub struct PalantirIndex {
    tier1: [HashMap<u64, Vec<Hash>>; 3],
    tier2: [HashMap<u64, Vec<Hash>>; 4],
    tier3: [HashMap<u64, Vec<Hash>>; 6],
    signatures: HashMap<Hash, ChunkSignature>,
    lru_order: VecDeque<Hash>,
    memory_budget: usize,
    count: usize,
}

impl PalantirIndex {
    pub fn new(memory_budget: usize) -> Self {
        Self {
            tier1: [HashMap::new(), HashMap::new(), HashMap::new()],
            tier2: [HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new()],
            tier3: [
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
            ],
            signatures: HashMap::new(),
            lru_order: VecDeque::new(),
            memory_budget,
            count: 0,
        }
    }

    pub fn insert(&mut self, hash: Hash, signature: &ChunkSignature) {
        self.enforce_budget();
        self.signatures.insert(hash, signature.clone());
        self.lru_order.push_back(hash);
        self.count += 1;
        for (i, sf) in signature.tier1.iter().enumerate() {
            self.tier1[i].entry(*sf).or_default().push(hash);
        }
        for (i, sf) in signature.tier2.iter().enumerate() {
            self.tier2[i].entry(*sf).or_default().push(hash);
        }
        for (i, sf) in signature.tier3.iter().enumerate() {
            self.tier3[i].entry(*sf).or_default().push(hash);
        }
    }

    /// Query for similar chunk. Returns best matching candidate or None.
    pub fn query(&mut self, hash: &Hash, signature: &ChunkSignature) -> Option<MatchCandidate> {
        let mut buf = Vec::with_capacity(64);
        if let Some(h) = self.query_tier(hash, &signature.tier1, &self.tier1, &mut buf) {
            self.touch(&h);
            return Some(MatchCandidate {
                hash: h,
                tier: SimilarityTier::High,
            });
        }
        if let Some(h) = self.query_tier(hash, &signature.tier2, &self.tier2, &mut buf) {
            self.touch(&h);
            return Some(MatchCandidate {
                hash: h,
                tier: SimilarityTier::Medium,
            });
        }
        if let Some(h) = self.query_tier(hash, &signature.tier3, &self.tier3, &mut buf) {
            self.touch(&h);
            return Some(MatchCandidate {
                hash: h,
                tier: SimilarityTier::Low,
            });
        }
        None
    }

    /// Mark a hash as recently used (moves to back of LRU queue).
    fn touch(&mut self, hash: &Hash) {
        // Remove and re-insert to move to back
        if let Some(pos) = self.lru_order.iter().position(|h| h == hash) {
            self.lru_order.remove(pos);
            self.lru_order.push_back(*hash);
        }
    }

    /// Query a single tier, returning best match (highest SF match count).
    fn query_tier<const N: usize>(
        &self,
        query_hash: &Hash,
        sfs: &[u64; N],
        tier_maps: &[HashMap<u64, Vec<Hash>>; N],
        out: &mut Vec<(Hash, usize)>,
    ) -> Option<Hash> {
        out.clear();

        for (i, sf) in sfs.iter().enumerate() {
            if let Some(entries) = tier_maps[i].get(sf) {
                for &entry_hash in entries {
                    if entry_hash == *query_hash {
                        continue;
                    }
                    if let Some(pos) = out.iter().position(|(h, _)| *h == entry_hash) {
                        out[pos].1 += 1;
                    } else {
                        out.push((entry_hash, 1));
                    }
                }
            }
        }

        let mut best: Option<(Hash, usize)> = None;
        for &(candidate_hash, match_count) in out.iter() {
            if self.signatures.contains_key(&candidate_hash) {
                match best {
                    Some((_, best_count)) if match_count > best_count => {
                        best = Some((candidate_hash, match_count));
                    }
                    None => {
                        best = Some((candidate_hash, match_count));
                    }
                    _ => {}
                }
            }
        }
        best.map(|(h, _)| h)
    }

    /// Remove an entry from the index.
    pub fn remove(&mut self, hash: &Hash) {
        if let Some(sig) = self.signatures.remove(hash) {
            for (i, sf) in sig.tier1.iter().enumerate() {
                if let Some(entries) = self.tier1[i].get_mut(sf) {
                    entries.retain(|h| h != hash);
                }
            }
            for (i, sf) in sig.tier2.iter().enumerate() {
                if let Some(entries) = self.tier2[i].get_mut(sf) {
                    entries.retain(|h| h != hash);
                }
            }
            for (i, sf) in sig.tier3.iter().enumerate() {
                if let Some(entries) = self.tier3[i].get_mut(sf) {
                    entries.retain(|h| h != hash);
                }
            }
            self.lru_order.retain(|h| h != hash);
            self.count = self.count.saturating_sub(1);
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn enforce_budget(&mut self) {
        while self.count > 0 && self.count >= self.memory_budget {
            if let Some(oldest) = self.lru_order.pop_front() {
                self.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::similarity::super_feature::extract_signature;

    fn test_hash(id: u8) -> Hash {
        let mut b = [0u8; 32];
        b[0] = id;
        Hash(b)
    }
    fn sig(data: &[u8]) -> ChunkSignature {
        extract_signature(data).unwrap()
    }

    #[test]
    fn test_empty_index() {
        let idx = PalantirIndex::new(10_000);
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        // query on empty index
        let mut q = PalantirIndex::new(10_000);
        assert!(q.query(&test_hash(0), &sig(&[0xABu8; 4096])).is_none());
    }

    #[test]
    fn test_insert_and_query_similar() {
        let mut idx = PalantirIndex::new(10_000);
        let a = vec![0xABu8; 4096];
        let s = sig(&a);
        idx.insert(test_hash(1), &s);
        let mut b = a.clone();
        b[100] = 0xFF;
        assert!(idx.query(&test_hash(2), &sig(&b)).is_some());
    }

    #[test]
    fn test_tier_matches_at_any_level() {
        let mut idx = PalantirIndex::new(10_000);
        ins(&mut idx, 1, &[0xABu8; 4096]);
        let mut m = vec![0xABu8; 4096];
        for i in 0..80 {
            m[i * 50] = 0xFF;
        }
        assert!(idx.query(&test_hash(2), &sig(&m)).is_some());
    }

    fn ins(idx: &mut PalantirIndex, id: u8, data: &[u8]) {
        let s = sig(data);
        idx.insert(test_hash(id), &s);
    }

    #[test]
    fn test_different_chunks_no_match() {
        let mut idx = PalantirIndex::new(10_000);
        ins(&mut idx, 1, &[0xABu8; 4096]);
        assert!(idx.query(&test_hash(2), &sig(&[0xCDu8; 4096])).is_none());
    }

    #[test]
    fn test_lru_eviction() {
        let mut idx = PalantirIndex::new(5);
        for i in 0..10 {
            #[allow(clippy::unnecessary_cast)]
            let d = vec![i as u8; 4096];
            let s = sig(&d);
            idx.insert(test_hash(i), &s);
        }
        assert!(idx.len() <= 5, "evicted to budget: {}", idx.len());
        let first_evicted = idx.query(&test_hash(0), &sig(&vec![0u8; 4096]));
        assert!(first_evicted.is_none(), "oldest entry should be evicted first");
    }

    #[test]
    fn test_remove() {
        let mut idx = PalantirIndex::new(10_000);
        let data = b"test data for removal test with enough bytes for a proper signature";
        let h = test_hash(99);
        let s = sig(data);
        idx.insert(h, &s);
        assert_eq!(idx.len(), 1);
        idx.remove(&h);
        assert!(idx.is_empty());
        assert!(
            idx.query(&test_hash(1), &sig(data)).is_none(),
            "removed entry not queryable"
        );
    }

    #[test]
    fn test_best_match_returned() {
        let mut idx = PalantirIndex::new(10_000);
        let base = vec![0xABu8; 4096];
        ins(&mut idx, 1, &base);
        let mut nb = base.clone();
        nb[2000] = 0xFF;
        ins(&mut idx, 2, &nb);
        let mut q = base.clone();
        q[100] = 0xFE;
        assert!(idx.query(&test_hash(3), &sig(&q)).is_some());
    }

    #[test]
    fn test_zero_and_ff_chunks() {
        let mut idx = PalantirIndex::new(10_000);
        ins(&mut idx, 1, &[0u8; 4096]);
        ins(&mut idx, 2, &[0xFFu8; 4096]);
        assert_eq!(idx.len(), 2);
        assert!(idx.query(&test_hash(3), &sig(&[0u8; 4096])).is_some());
        assert!(idx.query(&test_hash(4), &sig(&[0xFFu8; 4096])).is_some());
    }
}
