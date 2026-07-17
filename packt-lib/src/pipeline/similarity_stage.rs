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
    /// Create a new similarity stage with the given configuration.
    /// The Palantir index starts empty — use `set_index()` to restore from
    /// previously persisted signatures.
    #[must_use]
    pub fn new(config: SimilarityConfig) -> Self {
        let index = PalantirIndex::new(config.memory_budget);
        Self {
            index: Mutex::new(index),
            config,
        }
    }

    /// Replace the index with a pre-built one (e.g., rebuilt from stored signatures).
    pub fn set_index(&self, index: PalantirIndex) {
        if let Ok(mut guard) = self.index.lock() {
            *guard = index;
        }
    }

    pub fn process(&self, hash: Hash, data: Vec<u8>) -> SimilarityOutcome {
        let Some(signature) = extract_signature(&data) else {
            return SimilarityOutcome::TooSmall { hash, data };
        };

        let candidate = self
            .index
            .lock()
            .ok()
            .and_then(|mut index| index.query(&hash, &signature));

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
            if let Ok(mut index) = self.index.lock() {
                index.insert(hash, &signature);
            }
            SimilarityOutcome::Unique { hash, data }
        } else {
            if let Ok(mut index) = self.index.lock() {
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
        self.index.lock().map_or(0, |guard| guard.len())
    }
}
