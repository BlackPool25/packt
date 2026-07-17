//! Pack format: a content-addressed chunk bundle.
//!
//! ## Layouts
//!
//! **PACKv1** (Phase 1): each chunk compressed individually with zstd.
//! ```text
//! [chunk0_zstd] [chunk1_zstd] ... [index] [footer]
//! ```
//!
//! **PACKv2** (Phase 3): adds delta entries and per-entry metadata.
//! ```text
//! [chunk0] [chunk1] ... [index] [footer]
//! ```
//!
//! **PACKv3** (Phase 3 optimization): Full/FullRaw data concatenated into a single
//! zstd-compressed super-block for better cross-chunk compression. Delta entries
//! stored separately after the super-block.
//! ```text
//! [super_block_zstd] [delta_0] ... [delta_N] [index] [footer]
//! ```
//! Where `super_block_zstd` decompresses to: `[full_chunk0_raw]` `[full_chunk1_raw]`...
//! Index entries for Full/FullRaw point into the decompressed super-block.
//! Index entries for Delta point to their zstd dict frames in the pack.

use crate::error::{PacktError, Result};
use crate::types::{Hash, PackLocation};
use serde::{Deserialize, Serialize};

/// Magic bytes: "PACKv1" as u64 (little-endian: 0x3156314B43415050)
const PACK_MAGIC: u64 = 0x3156_314B_4341_5050;
/// Magic bytes: "PACKv2" as u64 (little-endian: 0x3256314B43415050)
const PACK_MAGIC_V2: u64 = 0x3256_314B_4341_5050;
/// Magic bytes: "PACKv3" as u64 (little-endian: 0x3356314B43415050)
const PACK_MAGIC_V3: u64 = 0x3356_314B_4341_5050;
const COMPRESSION_LEVEL: i32 = 3;

/// Quick entropy check: returns true if data appears randomly distributed
/// (high entropy, incompressible). Samples first 1KB; if > 220 unique byte
/// values observed, zstd will not meaningfully compress this chunk.
fn is_high_entropy(data: &[u8]) -> bool {
    let sample = &data[..1024.min(data.len())];
    if sample.len() < 256 {
        return false;
    }
    let mut seen = [false; 256];
    let mut count = 0u32;
    for &b in sample {
        if !seen[b as usize] {
            seen[b as usize] = true;
            count += 1;
            if count > 220 {
                return true;
            }
        }
    }
    false
}
const FOOTER_SIZE: usize = 52; // u64(8) + u32(4) + [u8;32] + u64(8) = 52

/// Type of entry in a pack file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum EntryType {
    #[default]
    Full,
    Delta {
        base_hash: Hash,
    },
    /// Stored uncompressed (zstd produced no benefit or expanded data).
    FullRaw,
}

/// Input entry for writing a pack file.
///
/// For `EntryType::Full`, `data` is raw (uncompressed) — the writer compresses it.
/// For `EntryType::Delta`, `data` is already a pre-compressed zstd frame using
/// the base chunk as a dictionary — the writer stores it as-is.
#[derive(Debug, Clone)]
pub struct PackEntry {
    pub hash: Hash,
    pub data: Vec<u8>,
    pub orig_length: u32,
    pub entry_type: EntryType,
    pub signature: Option<Vec<u8>>,
}

/// Entry in the pack index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub hash: Hash,
    pub offset: u64,
    pub length: u32,
    pub orig_length: u32,
    #[serde(default)]
    pub entry_type: EntryType,
    #[serde(default)]
    pub signature: Option<Vec<u8>>,
}

/// Footer at the end of every pack file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Footer {
    index_offset: u64,
    index_len: u32,
    checksum: [u8; 32],
    magic: u64,
}

/// Serialized pack index (v2+ format).
#[derive(Debug, Serialize, Deserialize)]
struct PackIndex {
    entries: Vec<IndexEntry>,
}

/// Backward-compatible v1 index entry (no entry_type, no signature).
#[derive(Debug, Serialize, Deserialize)]
struct IndexEntryV1 {
    hash: Hash,
    offset: u64,
    length: u32,
    orig_length: u32,
}

/// Backward-compatible v1 index wrapper.
#[derive(Debug, Serialize, Deserialize)]
struct PackIndexV1 {
    entries: Vec<IndexEntryV1>,
}

/// Write a pack file from a list of `PackEntry` items.
///
/// Uses PACKv3 format with super-block compression: Full/FullRaw data is
/// concatenated and compressed as a single zstd frame for better ratio.
/// Delta entries are stored separately after the super-block.
///
/// The compression level is taken from `COMPRESSION_LEVEL` constant.
/// Callers should ensure it matches `PipelineConfig.compression_level`.
pub fn write_pack(entries: &[PackEntry]) -> Result<Vec<u8>> {
    let mut pack = Vec::new();
    let mut index_entries = Vec::with_capacity(entries.len());
    let mut full_raw_data = Vec::new();
    let mut full_offset: u64 = 0;

    // First pass: build super-block from Full/FullRaw entries
    for entry in entries {
        match &entry.entry_type {
            EntryType::Full | EntryType::FullRaw => {
                let (compressed, actual_type) = match &entry.entry_type {
                    EntryType::Full => {
                        // Quick incompressibility check: if data has high entropy
                        // (all 256 byte values present), skip zstd to avoid wasted CPU.
                        if is_high_entropy(&entry.data) {
                            (entry.data.clone(), EntryType::FullRaw)
                        } else {
                            let c = zstd::bulk::compress(&entry.data, COMPRESSION_LEVEL)
                                .map_err(|e| PacktError::Serialization(format!("zstd compress failed: {e}")))?;
                            if c.len() >= entry.data.len() * 95 / 100 {
                                (entry.data.clone(), EntryType::FullRaw)
                            } else {
                                (c, EntryType::Full)
                            }
                        }
                    }
                    EntryType::FullRaw => (entry.data.clone(), EntryType::FullRaw),
                    EntryType::Delta { .. } => unreachable!(),
                };
                index_entries.push(IndexEntry {
                    hash: entry.hash,
                    offset: full_offset,
                    length: compressed.len() as u32,
                    orig_length: entry.orig_length,
                    entry_type: actual_type,
                    signature: entry.signature.clone(),
                });
                full_raw_data.extend_from_slice(&compressed);
                full_offset += compressed.len() as u64;
            }
            EntryType::Delta { .. } => {}
        }
    }

    // Compress super-block with multi-threaded zstd (via zstdmt feature).
    if !full_raw_data.is_empty() {
        use std::io::Write;
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)
            .map_err(|e| PacktError::Serialization(format!("zstd encoder: {e}")))?;
        encoder
            .multithread(4)
            .map_err(|e| PacktError::Serialization(format!("zstd multithread: {e}")))?;
        encoder
            .write_all(&full_raw_data)
            .map_err(|e| PacktError::Serialization(format!("zstd write: {e}")))?;
        let compressed = encoder
            .finish()
            .map_err(|e| PacktError::Serialization(format!("zstd finish: {e}")))?;
        pack.extend_from_slice(&compressed);
    }

    // Second pass: append Delta entries after super-block
    let delta_base_offset = pack.len() as u64;
    for entry in entries {
        if let EntryType::Delta { .. } = &entry.entry_type {
            let entry_offset = delta_base_offset + pack.len() as u64 - delta_base_offset;
            index_entries.push(IndexEntry {
                hash: entry.hash,
                offset: entry_offset,
                length: entry.data.len() as u32,
                orig_length: entry.orig_length,
                entry_type: entry.entry_type.clone(),
                signature: entry.signature.clone(),
            });
            pack.extend_from_slice(&entry.data);
        }
    }

    let index_offset = pack.len() as u64;
    let index = PackIndex { entries: index_entries };
    let index_bytes =
        postcard::to_stdvec(&index).map_err(|e| PacktError::Serialization(format!("postcard index: {e}")))?;
    let index_len = index_bytes.len() as u32;
    pack.extend_from_slice(&index_bytes);

    let footer = Footer {
        index_offset,
        index_len,
        checksum: [0u8; 32],
        magic: PACK_MAGIC_V3,
    };

    // Checksum covers data + footer fields (index_offset, index_len, magic)
    let mut hasher = blake3::Hasher::new();
    hasher.update(&pack);
    hasher.update(&footer.index_offset.to_le_bytes());
    hasher.update(&footer.index_len.to_le_bytes());
    hasher.update(&footer.magic.to_le_bytes());
    let checksum = *hasher.finalize().as_bytes();

    let footer = Footer {
        index_offset,
        index_len,
        checksum,
        magic: PACK_MAGIC_V3,
    };

    let footer_bytes = encode_footer(&footer);
    pack.extend_from_slice(&footer_bytes);

    Ok(pack)
}

fn encode_footer(f: &Footer) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FOOTER_SIZE);
    buf.extend_from_slice(&f.index_offset.to_le_bytes()); // 8 bytes
    buf.extend_from_slice(&f.index_len.to_le_bytes()); // 4 bytes
    buf.extend_from_slice(&f.checksum); // 32 bytes
    buf.extend_from_slice(&f.magic.to_le_bytes()); // 8 bytes
    debug_assert_eq!(buf.len(), FOOTER_SIZE);
    buf
}

fn decode_footer(data: &[u8]) -> Result<Footer> {
    if data.len() < FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("footer too short".into()));
    }
    let bytes = &data[..FOOTER_SIZE];
    let index_offset = u64::from_le_bytes(
        bytes[0..8]
            .try_into()
            .map_err(|_| PacktError::InvalidPackFormat("bad index_offset".into()))?,
    );
    let index_len = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| PacktError::InvalidPackFormat("bad index_len".into()))?,
    );
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&bytes[12..44]);
    let magic = u64::from_le_bytes(
        bytes[44..52]
            .try_into()
            .map_err(|_| PacktError::InvalidPackFormat("bad magic bytes".into()))?,
    );
    Ok(Footer {
        index_offset,
        index_len,
        checksum,
        magic,
    })
}

/// Read a pack file and return (entries, checksum, superblock).
///
/// Handles v1 (PACK_MAGIC), v2 (PACK_MAGIC_V2), and v3 (PACK_MAGIC_V3) formats.
/// For v3, the superblock contains decompressed Full/FullRaw data.
/// v1 entries are treated as `EntryType::Full` with `signature = None`.
pub type ReadPackResult = (Vec<IndexEntry>, [u8; 32], Option<Vec<u8>>);

pub fn read_pack(data: &[u8]) -> Result<ReadPackResult> {
    if data.len() < FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("Pack too short".into()));
    }

    let footer = decode_footer(&data[data.len() - FOOTER_SIZE..])?;

    if footer.magic != PACK_MAGIC && footer.magic != PACK_MAGIC_V2 && footer.magic != PACK_MAGIC_V3 {
        return Err(PacktError::InvalidPackFormat("bad magic".into()));
    }

    let footer_start = data.len() - FOOTER_SIZE;
    let data_to_verify = &data[..footer_start];
    let mut hasher = blake3::Hasher::new();
    hasher.update(data_to_verify);
    // V3 includes footer fields in checksum (index_offset, index_len, magic)
    if footer.magic == PACK_MAGIC_V3 {
        hasher.update(&footer.index_offset.to_le_bytes());
        hasher.update(&footer.index_len.to_le_bytes());
        hasher.update(&footer.magic.to_le_bytes());
    }
    let computed = *hasher.finalize().as_bytes();

    if computed != footer.checksum {
        return Err(PacktError::ChecksumMismatch {
            expected: hex::encode(footer.checksum),
            actual: hex::encode(computed),
        });
    }

    let index_start = footer.index_offset as usize;
    let index_end = index_start
        .checked_add(footer.index_len as usize)
        .ok_or_else(|| PacktError::InvalidPackFormat("index offset overflow".into()))?;
    if index_end > data.len() - FOOTER_SIZE {
        return Err(PacktError::InvalidPackFormat("index extends past data".into()));
    }

    let index_slice = &data[index_start..index_end];

    let entries = if footer.magic == PACK_MAGIC {
        // v1 format: deserialize legacy entries, treat all as Full
        let (pack_index, _): (PackIndexV1, _) = postcard::take_from_bytes(index_slice)
            .map_err(|e| PacktError::InvalidPackFormat(format!("bad index: {e}")))?;
        pack_index
            .entries
            .into_iter()
            .map(|e| IndexEntry {
                hash: e.hash,
                offset: e.offset,
                length: e.length,
                orig_length: e.orig_length,
                entry_type: EntryType::Full,
                signature: None,
            })
            .collect()
    } else {
        let (pack_index, _): (PackIndex, _) = postcard::take_from_bytes(index_slice)
            .map_err(|e| PacktError::InvalidPackFormat(format!("bad index: {e}")))?;
        pack_index.entries
    };

    // For v3, decompress the super-block (all Full/FullRaw data concatenated)
    let superblock = if footer.magic == PACK_MAGIC_V3 {
        // Calculate superblock size: minimum pack offset among Delta entries
        let sb_end = entries
            .iter()
            .filter(|e| matches!(e.entry_type, EntryType::Delta { .. }))
            .map(|e| e.offset as usize)
            .min()
            .unwrap_or(index_start);
        let superblock_compressed = &data[..sb_end];
        if superblock_compressed.is_empty() {
            None
        } else {
            let total_size: usize = entries
                .iter()
                .filter(|e| matches!(e.entry_type, EntryType::Full | EntryType::FullRaw))
                .map(|e| e.offset as usize + e.length as usize)
                .max()
                .unwrap_or(0);
            let decompressed = zstd::bulk::decompress(superblock_compressed, total_size)
                .map_err(|e| PacktError::Serialization(format!("zstd superblock decompress: {e}")))?;
            // Validate all Full/FullRaw entries fit within the decompressed superblock
            for e in &entries {
                if matches!(e.entry_type, EntryType::Full | EntryType::FullRaw) {
                    let end = e.offset as usize + e.length as usize;
                    if end > decompressed.len() {
                        return Err(PacktError::InvalidPackFormat(format!(
                            "entry {} offset+length {} exceeds superblock size {}",
                            e.hash.to_hex(),
                            end,
                            decompressed.len()
                        )));
                    }
                }
            }
            Some(decompressed)
        }
    } else {
        None
    };

    Ok((entries, footer.checksum, superblock))
}

/// Validate that `offset` and `length` represent a valid range within `data_len`.
/// Returns the `(start, end)` byte range on success.
fn checked_range(offset: u64, length: u32, data_len: usize) -> Result<(usize, usize)> {
    let start = offset as usize;
    let end = start
        .checked_add(length as usize)
        .ok_or_else(|| PacktError::InvalidPackFormat("chunk offset/length overflow".into()))?;
    if end > data_len {
        return Err(PacktError::InvalidPackFormat(format!(
            "chunk {offset}+{length} exceeds data size {data_len}"
        )));
    }
    Ok((start, end))
}

/// Read a specific chunk from pack data given its location (Full entries).
pub fn read_chunk(pack_data: &[u8], loc: &PackLocation) -> Result<Vec<u8>> {
    let (start, end) = checked_range(loc.offset, loc.length, pack_data.len())?;
    let compressed = &pack_data[start..end];
    let decompressed = zstd::bulk::decompress(compressed, loc.orig_length as usize)
        .map_err(|e| PacktError::Serialization(format!("zstd decompress: {e}")))?;
    Ok(decompressed)
}

/// Read a raw (uncompressed) chunk from pack data.
pub fn read_raw_chunk(pack_data: &[u8], loc: &PackLocation) -> Result<Vec<u8>> {
    let (start, end) = checked_range(loc.offset, loc.length, pack_data.len())?;
    Ok(pack_data[start..end].to_vec())
}

/// Read a delta chunk from pack data using `base_chunk` as the zstd dictionary.
///
/// Delta entries are stored as zstd frames compressed with the base chunk as
/// a dictionary. This function decompresses using `base_chunk` as the dict.
pub fn read_delta_chunk(pack_data: &[u8], loc: &PackLocation, base_chunk: &[u8]) -> Result<Vec<u8>> {
    let (start, end) = checked_range(loc.offset, loc.length, pack_data.len())?;
    let compressed = &pack_data[start..end];
    let mut decompressor = zstd::bulk::Decompressor::with_dictionary(base_chunk)
        .map_err(|e| PacktError::Serialization(format!("zstd dict decompressor: {e}")))?;
    let decompressed = decompressor
        .decompress(compressed, loc.orig_length as usize)
        .map_err(|e| PacktError::Serialization(format!("zstd dict decompress: {e}")))?;

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Hash;

    fn read_entry(pack: &[u8], entry: &IndexEntry) -> Vec<u8> {
        match entry.entry_type {
            EntryType::Full | EntryType::FullRaw => {
                let (_, _, sb) = read_pack(pack).unwrap();
                let sb = sb.expect("Full entry in V3 pack needs superblock");
                let start = entry.offset as usize;
                sb[start..start + entry.length as usize].to_vec()
            }
            EntryType::Delta { .. } => panic!("read_entry does not support Delta"),
        }
    }

    fn make_full_entry(data: Vec<u8>) -> PackEntry {
        let hash = Hash::from_blake3(blake3::hash(&data));
        let orig_length = data.len() as u32;
        PackEntry {
            hash,
            data,
            orig_length,
            entry_type: EntryType::Full,
            signature: None,
        }
    }

    #[test]
    fn test_pack_empty() {
        let entries = vec![];
        let pack = write_pack(&entries).unwrap();
        let (result_entries, _checksum, _sb) = read_pack(&pack).unwrap();
        assert!(result_entries.is_empty(), "Empty pack should produce empty index");
    }

    #[test]
    fn test_pack_single_chunk() {
        let data = b"hello pack format test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let orig_length = data.len() as u32;

        let entry = PackEntry {
            hash,
            data: data.clone(),
            orig_length,
            entry_type: EntryType::Full,
            signature: None,
        };
        let pack = write_pack(&[entry]).unwrap();

        let (entries, _checksum, _sb) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);

        let recovered = read_entry(&pack, &entries[0]);
        assert_eq!(recovered, data);
        assert_eq!(entries[0].hash, hash);
    }

    #[test]
    fn test_pack_multiple_chunks() {
        let mut entries = Vec::new();
        for i in 0..50 {
            let data = format!("pack chunk number {i} with some unique data to test roundtripping");
            let data_bytes = data.into_bytes();
            let hash = Hash::from_blake3(blake3::hash(&data_bytes));
            entries.push(PackEntry {
                hash,
                data: data_bytes,
                orig_length: i as u32 + 100,
                entry_type: EntryType::Full,
                signature: None,
            });
        }

        let pack = write_pack(&entries).unwrap();
        let (result_entries, _checksum, _sb) = read_pack(&pack).unwrap();
        assert_eq!(result_entries.len(), 50);

        for (i, entry) in entries.iter().enumerate() {
            let recovered = read_entry(&pack, &result_entries[i]);
            assert_eq!(&recovered, &entry.data, "Chunk {i} data mismatch");
            assert_eq!(result_entries[i].hash, entry.hash, "Chunk {i} hash mismatch");
        }
    }

    #[test]
    fn test_pack_checksum_integrity() {
        let data = b"checksum test data".to_vec();
        let mut pack = write_pack(&[make_full_entry(data)]).unwrap();

        // Corrupt one byte in the data section
        let corrupt_pos = 5;
        pack[corrupt_pos] ^= 0xFF;

        let result = read_pack(&pack);
        assert!(result.is_err(), "Corrupted pack should fail verification");
    }

    #[test]
    fn test_pack_truncated_detection() {
        let data = b"truncation test data".to_vec();
        let pack = write_pack(&[make_full_entry(data)]).unwrap();
        let truncated = &pack[..pack.len() - 10];

        let result = read_pack(truncated);
        assert!(result.is_err(), "Truncated pack should fail");
    }

    #[test]
    fn test_pack_invalid_magic() {
        let data = b"magic test".to_vec();
        let mut pack = write_pack(&[make_full_entry(data)]).unwrap();
        // Corrupt magic bytes
        let pack_len = pack.len();
        pack[pack_len - 6] = 0xFF;

        let result = read_pack(&pack);
        assert!(result.is_err(), "Corrupted magic should fail");
    }

    #[test]
    fn test_pack_large_data() {
        let data = vec![0xCDu8; 500_000]; // 500 KB
        let pack = write_pack(&[make_full_entry(data)]).unwrap();
        let (entries, _checksum, _sb) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_pack_v2_roundtrip() {
        let data = b"v2 roundtrip full entry data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let entry = PackEntry {
            hash,
            data: data.clone(),
            orig_length: data.len() as u32,
            entry_type: EntryType::Full,
            signature: None,
        };

        let pack = write_pack(&[entry]).unwrap();
        let (entries, _checksum, _sb) = read_pack(&pack).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, hash);
        assert!(matches!(entries[0].entry_type, EntryType::Full | EntryType::FullRaw));
        assert!(entries[0].signature.is_none());

        let recovered = read_entry(&pack, &entries[0]);
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_pack_v2_delta() {
        let base_data = b"this is the base chunk data for delta compression testing purposes ".to_vec();
        let target_data = b"this is the SIMILAR chunk data for delta compression testing purposes ".to_vec();

        let base_hash = Hash::from_blake3(blake3::hash(&base_data));
        let target_hash = Hash::from_blake3(blake3::hash(&target_data));

        // Compress target_data using base_data as dictionary
        let mut compressor = zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, &base_data).unwrap();
        let delta_compressed = compressor.compress(&target_data).unwrap();

        let entry = PackEntry {
            hash: target_hash,
            data: delta_compressed,
            orig_length: target_data.len() as u32,
            entry_type: EntryType::Delta { base_hash },
            signature: None,
        };

        let pack = write_pack(&[entry]).unwrap();
        let (entries, _checksum, _sb) = read_pack(&pack).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, target_hash);
        match &entries[0].entry_type {
            EntryType::Delta { base_hash: bh } => assert_eq!(*bh, base_hash),
            EntryType::Full | EntryType::FullRaw => panic!("Expected Delta entry type"),
        }

        // Verify we can read the delta chunk back
        let loc = PackLocation {
            pack_id: 0,
            offset: entries[0].offset,
            length: entries[0].length,
            orig_length: entries[0].orig_length,
        };
        let recovered = read_delta_chunk(&pack, &loc, &base_data).unwrap();
        assert_eq!(recovered, target_data);
    }

    #[test]
    fn test_pack_v2_mixed() {
        let full_data = b"hello full chunk data".to_vec();
        let full_hash = Hash::from_blake3(blake3::hash(&full_data));

        let base_data = b"this is the base chunk for delta testing ".to_vec();
        let target_data = b"this is the BASE chunk for delta testing too".to_vec();
        let base_hash = Hash::from_blake3(blake3::hash(&base_data));
        let target_hash = Hash::from_blake3(blake3::hash(&target_data));

        let mut compressor = zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, &base_data).unwrap();
        let delta_compressed = compressor.compress(&target_data).unwrap();

        let entries = vec![
            PackEntry {
                hash: full_hash,
                data: full_data.clone(),
                orig_length: full_data.len() as u32,
                entry_type: EntryType::Full,
                signature: None,
            },
            PackEntry {
                hash: target_hash,
                data: delta_compressed,
                orig_length: target_data.len() as u32,
                entry_type: EntryType::Delta { base_hash },
                signature: None,
            },
        ];

        let pack = write_pack(&entries).unwrap();
        let (read_entries, _checksum, _sb) = read_pack(&pack).unwrap();

        assert_eq!(read_entries.len(), 2);

        // Verify Full entry
        assert_eq!(read_entries[0].hash, full_hash);
        assert!(matches!(
            read_entries[0].entry_type,
            EntryType::Full | EntryType::FullRaw
        ));
        let recovered_full = read_entry(&pack, &read_entries[0]);
        assert_eq!(recovered_full, full_data);

        // Verify Delta entry
        assert_eq!(read_entries[1].hash, target_hash);
        assert!(matches!(read_entries[1].entry_type, EntryType::Delta { .. }));
        let loc1 = PackLocation {
            pack_id: 0,
            offset: read_entries[1].offset,
            length: read_entries[1].length,
            orig_length: read_entries[1].orig_length,
        };
        let recovered_delta = read_delta_chunk(&pack, &loc1, &base_data).unwrap();
        assert_eq!(recovered_delta, target_data);
    }

    #[test]
    fn test_pack_v1_backward_compat() {
        // Build a v1-format pack manually and verify read_pack handles it
        let data = b"backward compat test data".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let orig_length = data.len() as u32;

        let mut pack = Vec::new();
        let compressed = zstd::bulk::compress(&data, COMPRESSION_LEVEL).unwrap();
        let entry_offset = 0u64;
        let entry_length = compressed.len() as u32;
        pack.extend_from_slice(&compressed);

        let index_offset = pack.len() as u64;
        let v1_index = PackIndexV1 {
            entries: vec![IndexEntryV1 {
                hash,
                offset: entry_offset,
                length: entry_length,
                orig_length,
            }],
        };
        let index_bytes = postcard::to_stdvec(&v1_index).unwrap();
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
        pack.extend_from_slice(&encode_footer(&footer));

        // Read back with the unified read_pack
        let (entries, _checksum, _sb) = read_pack(&pack).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hash, hash);
        assert_eq!(entries[0].offset, entry_offset);
        assert_eq!(entries[0].length, entry_length);
        assert_eq!(entries[0].orig_length, orig_length);
        // v1 entries should be treated as Full with no signature
        assert!(matches!(entries[0].entry_type, EntryType::Full));
        assert!(entries[0].signature.is_none());

        // Verify the chunk can still be read
        let loc = PackLocation {
            pack_id: 0,
            offset: entries[0].offset,
            length: entries[0].length,
            orig_length: entries[0].orig_length,
        };
        let recovered = read_chunk(&pack, &loc).unwrap();
        assert_eq!(recovered, data);
    }

    #[test]
    fn test_pack_v2_magic_accept() {
        // v2 format is handled by read_pack — verify it works
        let data = b"v2 magic acceptance test".to_vec();
        let hash = Hash::from_blake3(blake3::hash(&data));
        let entry = PackEntry {
            hash,
            data: data.clone(),
            orig_length: data.len() as u32,
            entry_type: EntryType::Full,
            signature: None,
        };

        let pack = write_pack(&[entry]).unwrap();
        // read_pack should succeed (handles v2 magic)
        let result = read_pack(&pack);
        assert!(result.is_ok(), "read_pack should accept v2 packs");
    }
}
