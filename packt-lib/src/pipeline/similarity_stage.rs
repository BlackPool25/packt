use crate::similarity::SimilarityConfig;
use crate::similarity::palantir::{PalantirIndex, SimilarityTier};
use crate::similarity::super_feature::extract_signature;
use crate::types::Hash;
use std::sync::Mutex;

/// Similarity detection stage using Palantir hierarchical super-features.
pub struct SimilarityStage {
    index: Mutex<PalantirIndex>,
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
    #[must_use]
    pub fn new(config: SimilarityConfig) -> Self {
        let index = PalantirIndex::new(config.memory_budget);
        Self {
            index: Mutex::new(index),
            config,
        }
    }

    pub fn process(&self, hash: Hash, data: Vec<u8>) -> SimilarityOutcome {
        let Some(signature) = extract_signature(&data) else {
            return SimilarityOutcome::TooSmall { hash, data };
        };

        let candidate = {
            let mut index = self.index.lock().expect("Palantir index lock poisoned");
            index.query(&hash, &signature)
        };

        if let Some(candidate) = candidate {
            // Filter by threshold: only accept tiers that meet the configured threshold
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
            // Tier didn't meet threshold, treat as unique but still insert
            {
                let mut index = self.index.lock().expect("Palantir index lock poisoned");
                index.insert(hash, &signature);
            }
            SimilarityOutcome::Unique { hash, data }
        } else {
            {
                let mut index = self.index.lock().expect("Palantir index lock poisoned");
                index.insert(hash, &signature);
            }
            SimilarityOutcome::Unique { hash, data }
        }
    }

    #[must_use]
    pub const fn config(&self) -> &SimilarityConfig {
        &self.config
    }

    #[must_use]
    pub fn index_size(&self) -> usize {
        self.index.lock().expect("Palantir index lock poisoned").len()
    }
}
