use crate::similarity::SimilarityConfig;
use crate::similarity::palantir::{ShardedPalantirIndex, SimilarityTier};
use crate::similarity::super_feature::extract_signature;
use crate::types::Hash;

/// Similarity detection stage using Palantir hierarchical super-features.
///
/// Internally sharded (4 shards by hash prefix) for concurrent access.
pub struct SimilarityStage {
    index: ShardedPalantirIndex,
    config: SimilarityConfig,
}

/// Outcome from the similarity stage.
#[derive(Debug, Clone)]
pub enum SimilarityOutcome {
    Unique {
        hash: Hash,
        data: Vec<u8>,
    },
    NearDuplicate {
        hash: Hash,
        data: Vec<u8>,
        similar_to: Hash,
        tier: SimilarityTier,
    },
    TooSmall {
        hash: Hash,
        data: Vec<u8>,
    },
}

impl SimilarityStage {
    /// Create a new similarity stage with the given configuration.
    /// The Palantir index starts empty — use `set_index()` to restore from
    /// previously persisted signatures.
    #[must_use]
    pub fn new(config: SimilarityConfig) -> Self {
        Self {
            index: ShardedPalantirIndex::new(config.memory_budget),
            config,
        }
    }

    /// Replace the index with a pre-built one (e.g., rebuilt from stored signatures).
    pub fn set_index(&self, index: &ShardedPalantirIndex) {
        for (hash, sig) in index.export_entries() {
            self.index.insert(hash, &sig);
        }
    }

    /// Replace the index by rebuilding from a sorted entry list.
    pub fn rebuild_index(&self, entries: Vec<(Hash, crate::similarity::super_feature::ChunkSignature)>) {
        self.index.rebuild(entries);
    }

    pub fn process(&self, hash: Hash, data: Vec<u8>) -> SimilarityOutcome {
        let Some(signature) = extract_signature(&data) else {
            return SimilarityOutcome::TooSmall { hash, data };
        };

        let candidate = self.index.query(&hash, &signature);

        if let Some(candidate) = candidate {
            let tier_ok = match candidate.tier {
                SimilarityTier::High => self.config.threshold <= 1.0,
                SimilarityTier::Medium => self.config.threshold <= 0.85,
                SimilarityTier::Low => self.config.threshold <= 0.70,
            };
            if tier_ok {
                return SimilarityOutcome::NearDuplicate {
                    hash,
                    data,
                    similar_to: candidate.hash,
                    tier: candidate.tier,
                };
            }
            self.index.insert(hash, &signature);
            SimilarityOutcome::Unique { hash, data }
        } else {
            self.index.insert(hash, &signature);
            SimilarityOutcome::Unique { hash, data }
        }
    }

    #[must_use]
    pub const fn config(&self) -> &SimilarityConfig {
        &self.config
    }

    #[must_use]
    pub fn index_size(&self) -> usize {
        self.index.len()
    }
}
