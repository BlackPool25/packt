#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

pub mod chunking;
pub mod error;
pub mod hash;
pub mod index;
pub mod pipeline;
pub mod store;
pub mod types;
// pub mod util; // Removed: BufferPool was unused; re-add when needed

pub use error::{PacktError, Result as PacktResult};
pub use types::{Chunk, ChunkConfig, Hash, PackLocation};
