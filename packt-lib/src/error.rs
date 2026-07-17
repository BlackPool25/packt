use thiserror::Error;

#[derive(Error, Debug)]
pub enum PacktError {
    #[error("IO error: {context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Invalid pack format: {0}")]
    InvalidPackFormat(String),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Chunk not found: {0}")]
    ChunkNotFound(String),

    #[error("Store corrupted: {0}")]
    StoreCorrupted(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Pipeline error: {0}")]
    Pipeline(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Similarity index error: {0}")]
    Similarity(String),

    /// Cloud storage error (S3/GCS). Only available with the "cloud" feature.
    #[cfg(feature = "cloud")]
    #[error("Cloud storage error: {context}")]
    Cloud {
        context: String,
        #[source]
        source: opendal::Error,
    },
}

impl From<std::io::Error> for PacktError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            context: err.to_string(),
            source: err,
        }
    }
}

/// Convert OpenDAL errors into PacktError::Cloud.
#[cfg(feature = "cloud")]
impl From<opendal::Error> for PacktError {
    fn from(err: opendal::Error) -> Self {
        Self::Cloud {
            context: err.to_string(),
            source: err,
        }
    }
}

pub type Result<T> = std::result::Result<T, PacktError>;
