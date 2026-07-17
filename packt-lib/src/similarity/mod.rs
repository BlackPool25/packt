pub mod palantir;
pub mod super_feature;

use crate::types::Hash;

/// Configuration for the Palantir hierarchical similarity detector.
#[derive(Debug, Clone, Copy)]
pub struct SimilarityConfig {
    /// Similarity threshold (0.0-1.0). Only relevant for CLI display.
    /// Tier matching is always hierarchical:
    ///   ≥95% → Tier 1  |  ≥85% → Tier 2  |  ≥70% → Tier 3
    pub threshold: f64,
    /// Maximum number of entries in the similarity index before LRU eviction.
    pub memory_budget: usize,
    /// Enable the false positive filter (head+tail comparison).
    pub enable_fpr: bool,
}

impl Default for SimilarityConfig {
    fn default() -> Self {
        Self {
            threshold: 0.7,
            memory_budget: 1_000_000,
            enable_fpr: true,
        }
    }
}

/// Result of a similarity check for a chunk.
#[derive(Debug, Clone)]
pub enum SimilarityResult {
    /// No similar chunk found — store as unique.
    Unique,
    /// Near-duplicate detected.
    NearDuplicate {
        /// Hash of the similar (base) chunk.
        similar_to: Hash,
        /// Similarity tier: High (≥95%), Medium (≥85%), Low (≥70%).
        tier: palantir::SimilarityTier,
    },
    /// Chunk too small for similarity detection (< 64 bytes).
    TooSmall,
}
