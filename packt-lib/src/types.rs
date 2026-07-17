use serde::{Deserialize, Serialize};
use std::fmt;

/// A BLAKE3 hash used as content identifier (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// Create a Hash from a blake3::Hash output.
    #[must_use]
    pub fn from_blake3(hash: blake3::Hash) -> Self {
        Self(*hash.as_bytes())
    }

    /// Create a Hash from raw 32 bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Format as lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// Parse from hex string.
    ///
    /// # Errors
    /// Returns error if hex string is not 64 characters or contains invalid hex.
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let bytes = hex_decode(s).map_err(|e| format!("Invalid hex: {e}"))?;
        if bytes.len() != 32 {
            return Err("Hex string must decode to 32 bytes".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(char::from(HEX_CHARS[(b >> 4) as usize]));
        s.push(char::from(HEX_CHARS[(b & 0x0F) as usize]));
    }
    s
}

const HEX_CHARS: &[u8] = b"0123456789abcdef";

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("Hex string must have even length".to_string());
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = nybble(chunk[0])?;
        let lo = nybble(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn nybble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("Invalid hex character: {b}")),
    }
}

/// Configuration for CDC chunking.
#[derive(Debug, Clone, Copy)]
pub struct ChunkConfig {
    pub min_size: usize,
    pub avg_size: usize,
    pub max_size: usize,
}

impl ChunkConfig {
    /// Default config: min=16KB, avg=32KB, max=128KB.
    #[must_use]
    pub fn default_32k() -> Self {
        Self {
            min_size: 16_384,  // 16 KB
            avg_size: 32_768,  // 32 KB
            max_size: 131_072, // 128 KB
        }
    }

    /// Validate the configuration.
    #[must_use]
    pub fn validate(&self) -> bool {
        self.min_size >= 64
            && self.avg_size >= self.min_size * 2
            && self.max_size >= self.avg_size * 2
            && self.max_size <= 1_048_576 // 1 MB max
    }
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self::default_32k()
    }
}

/// A single chunk produced by a chunker.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub offset: u64,
    pub length: u32,
    pub data: Vec<u8>,
}

/// Location of a chunk within a pack file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackLocation {
    pub pack_id: u32,
    pub offset: u64,
    pub length: u32,
    pub orig_length: u32,
}
