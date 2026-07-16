//! Pack format: a content-addressed chunk bundle.
//!
//! Layout:
//! ```text
//! [Chunk 0: zstd-compressed data]
//! [Chunk 1: zstd-compressed data]
//! ...
//! [Chunk N: zstd-compressed data]
//! [Index: bincode-serialized Vec<IndexEntry>]
//! [Footer: index_offset(u64), index_len(u32), checksum([u8;32]), magic(b"PACKv1")]
//! ```

use crate::error::{PacktError, Result};
use crate::types::{Hash, PackLocation};
use serde::{Deserialize, Serialize};

/// Magic bytes: "PACKv1" as u64 (little-endian: 0x3156314B43415050)
const PACK_MAGIC: u64 = 0x3156_314B_4341_5050;
const COMPRESSION_LEVEL: i32 = 3;
const FOOTER_SIZE: usize = 52; // u64(8) + u32(4) + [u8;32] + u64(8) = 52

/// Entry in the pack index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub hash: Hash,
    pub offset: u64,
    pub length: u32,
    pub orig_length: u32,
}

/// Footer at the end of every pack file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Footer {
    index_offset: u64,
    index_len: u32,
    checksum: [u8; 32],
    magic: u64,
}

/// Serialized pack index.
#[derive(Debug, Serialize, Deserialize)]
struct PackIndex {
    entries: Vec<IndexEntry>,
}

/// Write a pack file from a list of (hash, data, original_length) tuples.
pub fn write_pack(chunks: &[(Hash, Vec<u8>, u32)]) -> Result<Vec<u8>> {
    let mut pack = Vec::new();
    let mut entries = Vec::with_capacity(chunks.len());

    for (hash, data, orig_length) in chunks {
        let entry_offset = pack.len() as u64;

        let compressed = zstd::bulk::compress(data, COMPRESSION_LEVEL)
            .map_err(|e| PacktError::Serialization(format!("zstd compress failed: {e}")))?;

        let entry_length = compressed.len() as u32;
        pack.extend_from_slice(&compressed);

        entries.push(IndexEntry {
            hash: *hash,
            offset: entry_offset,
            length: entry_length,
            orig_length: *orig_length,
        });
    }

    let index_offset = pack.len() as u64;
    let index = PackIndex { entries };
    let index_bytes = postcard::to_stdvec(&index)
        .map_err(|e| PacktError::Serialization(format!("postcard index: {e}")))?;
    let index_len = index_bytes.len() as u32;
    pack.extend_from_slice(&index_bytes);

    let mut hasher = blake3::Hasher::new();
    hasher.update(&pack);
    let checksum = *hasher.finalize().as_bytes();

    let footer = Footer {
        index_offset,
        index_len,
        checksum,
        magic: PACK_MAGIC,
    };

    let footer_bytes = encode_footer(&footer);
    pack.extend_from_slice(&footer_bytes);

    Ok(pack)
}

fn encode_footer(f: &Footer) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FOOTER_SIZE);
    buf.extend_from_slice(&f.index_offset.to_le_bytes()); // 8 bytes
    buf.extend_from_slice(&f.index_len.to_le_bytes());     // 4 bytes
    buf.extend_from_slice(&f.checksum);                     // 32 bytes
    buf.extend_from_slice(&f.magic.to_le_bytes());          // 8 bytes
    debug_assert_eq!(buf.len(), FOOTER_SIZE);
    buf
}

fn decode_footer(data: &[u8]) -> Result<Footer> {
    if data.len() < FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("footer too short".into()));
    }
    let bytes = &data[..FOOTER_SIZE];
    let index_offset = u64::from_le_bytes(
        bytes[0..8].try_into().map_err(|_| PacktError::InvalidPackFormat("bad index_offset".into()))?
    );
    let index_len = u32::from_le_bytes(
        bytes[8..12].try_into().map_err(|_| PacktError::InvalidPackFormat("bad index_len".into()))?
    );
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&bytes[12..44]);
    let magic = u64::from_le_bytes(
        bytes[44..52].try_into().map_err(|_| PacktError::InvalidPackFormat("bad magic bytes".into()))?
    );
    Ok(Footer { index_offset, index_len, checksum, magic })
}

/// Read a pack file and return (entries, checksum).
pub fn read_pack(data: &[u8]) -> Result<(Vec<IndexEntry>, [u8; 32])> {
    if data.len() < FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("Pack too short".into()));
    }

    let footer = decode_footer(&data[data.len() - FOOTER_SIZE..])?;

    if footer.magic != PACK_MAGIC {
        return Err(PacktError::InvalidPackFormat("bad magic".into()));
    }

    let footer_start = data.len() - FOOTER_SIZE;
    let data_to_verify = &data[..footer_start];
    let mut hasher = blake3::Hasher::new();
    hasher.update(data_to_verify);
    let computed = *hasher.finalize().as_bytes();

    if computed != footer.checksum {
        return Err(PacktError::ChecksumMismatch {
            expected: hex::encode(footer.checksum),
            actual: hex::encode(computed),
        });
    }

    let index_start = footer.index_offset as usize;
    let index_end = index_start + footer.index_len as usize;
    if index_end > data.len() - FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("index extends past data".into()));
    }

    let index_slice = &data[index_start..index_end];
    let (pack_index, _): (PackIndex, _) = postcard::take_from_bytes(index_slice)
        .map_err(|e| PacktError::InvalidPackFormat(format!("bad index: {e}")))?;

    Ok((pack_index.entries, footer.checksum))
}

/// Read a specific chunk from pack data given its location.
pub fn read_chunk(pack_data: &[u8], loc: &PackLocation) -> Result<Vec<u8>> {
    let start = loc.offset as usize;
    let end = start + loc.length as usize;
    if end > pack_data.len() {
        return Err(PacktError::InvalidPackFormat(format!(
            "chunk {}+{} exceeds pack size {}",
            loc.offset, loc.length, pack_data.len()
        )));
    }

    let compressed = &pack_data[start..end];
    let decompressed = zstd::bulk::decompress(compressed, loc.orig_length as usize)
        .map_err(|e| PacktError::Serialization(format!("zstd decompress: {e}")))?;

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Hash;

    #[test]
    fn test_pack_empty() {
        let chunks = vec![];
        let pack = write_pack(&chunks).unwrap();
        let (entries, _checksum) = read_pack(&pack).unwrap();
        assert!(entries.is_empty(), "Empty pack should produce empty index");
    }

    #[test]
    fn test_pack_single_chunk() {
        let data = b"hello pack format test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let orig_length = data.len() as u32;

        let chunks = vec![(hash, data.clone(), orig_length)];
        let pack = write_pack(&chunks).unwrap();

        let (entries, _checksum) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);

        // Verify we can read the chunk back
        let loc = PackLocation {
            pack_id: 0,
            offset: entries[0].offset,
            length: entries[0].length,
            orig_length: entries[0].orig_length,
        };
        let recovered = read_chunk(&pack, &loc).unwrap();
        assert_eq!(recovered, data);
        assert_eq!(entries[0].hash, hash);
    }

    #[test]
    fn test_pack_multiple_chunks() {
        let mut chunks = Vec::new();
        for i in 0..50 {
            let data = format!("pack chunk number {i} with some unique data to test roundtripping");
            let data_bytes = data.into_bytes();
            let hash = Hash::from_blake3(blake3::hash(&data_bytes));
            chunks.push((hash, data_bytes, chunks.len() as u32 + 100));
        }

        let pack = write_pack(&chunks).unwrap();
        let (entries, _checksum) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 50);

        // Verify all chunks round-trip
        for (i, (hash, data, _orig_len)) in chunks.iter().enumerate() {
            let loc = PackLocation {
                pack_id: 0,
                offset: entries[i].offset,
                length: entries[i].length,
                orig_length: entries[i].orig_length,
            };
            let recovered = read_chunk(&pack, &loc).unwrap();
            assert_eq!(&recovered, data, "Chunk {i} data mismatch");
            assert_eq!(entries[i].hash, *hash, "Chunk {i} hash mismatch");
        }
    }

    #[test]
    fn test_pack_checksum_integrity() {
        let data = b"checksum test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let chunks = vec![(hash, data, 18u32)];

        let mut pack = write_pack(&chunks).unwrap();

        // Corrupt one byte in the data section
        let corrupt_pos = 5;
        pack[corrupt_pos] ^= 0xFF;

        let result = read_pack(&pack);
        assert!(result.is_err(), "Corrupted pack should fail verification");
    }

    #[test]
    fn test_pack_truncated_detection() {
        let data = b"truncation test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let chunks = vec![(hash, data, 20u32)];

        let pack = write_pack(&chunks).unwrap();
        let truncated = &pack[..pack.len() - 10];

        let result = read_pack(truncated);
        assert!(result.is_err(), "Truncated pack should fail");
    }

    #[test]
    fn test_pack_invalid_magic() {
        let data = b"magic test".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let chunks = vec![(hash, data, 10u32)];

        let mut pack = write_pack(&chunks).unwrap();
        // Corrupt magic bytes
        let pack_len = pack.len();
        pack[pack_len - 6] = 0xFF;

        let result = read_pack(&pack);
        assert!(result.is_err(), "Corrupted magic should fail");
    }

    #[test]
    fn test_pack_large_data() {
        let data = vec![0xCDu8; 500_000]; // 500 KB
        let hash = Hash::from_blake3(blake3::hash(&data));
        let chunks = vec![(hash, data, 500_000u32)];

        let pack = write_pack(&chunks).unwrap();
        let (entries, _checksum) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);
    }
}
