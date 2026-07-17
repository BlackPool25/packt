use anyhow::Result;
use packt_lib::store::pack::{self, EntryType, IndexEntry};
use packt_lib::types::{Hash, PackLocation};
use std::path::Path;

type PackEntryMap = Vec<(Vec<u8>, Vec<IndexEntry>, Option<Vec<u8>>)>;

/// Build a hash → (pack_data, entries, superblock) map from all packs.
fn build_entry_map(packs_dir: &Path) -> Result<PackEntryMap> {
    let mut pack_files: Vec<_> = std::fs::read_dir(packs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "pack"))
        .collect();
    pack_files.sort_by_key(|e| e.file_name());

    let mut packs = Vec::new();
    for entry in &pack_files {
        let data = std::fs::read(entry.path())?;
        match pack::read_pack(&data) {
            Ok((entries, _checksum, superblock)) => {
                packs.push((data, entries, superblock));
            }
            Err(e) => {
                anyhow::bail!("Pack {} is corrupt: {e}", entry.path().display());
            }
        }
    }
    Ok(packs)
}

/// Read a chunk from any pack given its hash.
fn read_chunk_by_hash(packs: &PackEntryMap, hash: &Hash) -> Result<Vec<u8>> {
    for (pack_data, entries, superblock) in packs {
        for entry in entries {
            if entry.hash == *hash {
                let loc = PackLocation {
                    pack_id: 0,
                    offset: entry.offset,
                    length: entry.length,
                    orig_length: entry.orig_length,
                };
                return match &entry.entry_type {
                    EntryType::Full => {
                        if let Some(sb) = superblock {
                            let data = &sb[loc.offset as usize..][..loc.length as usize];
                            zstd::bulk::decompress(data, loc.orig_length as usize).map_err(|e| anyhow::anyhow!("{e}"))
                        } else {
                            pack::read_chunk(pack_data, &loc).map_err(|e| anyhow::anyhow!("{e}"))
                        }
                    }
                    EntryType::FullRaw => {
                        if let Some(sb) = superblock {
                            Ok(sb[loc.offset as usize..][..loc.length as usize].to_vec())
                        } else {
                            pack::read_raw_chunk(pack_data, &loc).map_err(|e| anyhow::anyhow!("{e}"))
                        }
                    }
                    EntryType::Delta { base_hash } => {
                        let base_chunk = read_chunk_by_hash(packs, base_hash)?;
                        pack::read_delta_chunk(pack_data, &loc, &base_chunk).map_err(|e| anyhow::anyhow!("{e}"))
                    }
                };
            }
        }
    }
    anyhow::bail!("Chunk {} not found across all packs", hash.to_hex());
}

pub fn run_verify(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let packs_dir = path.join("packs");
    if !packs_dir.exists() {
        anyhow::bail!("No packs directory found at: {}", packs_dir.display());
    }

    let all_packs = build_entry_map(&packs_dir)?;

    if all_packs.is_empty() {
        println!("No pack files to verify.");
        return Ok(());
    }

    let mut verified = 0u64;
    let mut failed = 0u64;

    for (pack_data, entries, superblock) in &all_packs {
        let checksum = blake3::hash(pack_data);
        eprintln!(
            "  ✓ pack ({} chunks, checksum: {})",
            entries.len(),
            hex::encode(*checksum.as_bytes())
        );
        verified += 1;

        for entry in entries {
            let loc = PackLocation {
                pack_id: 0,
                offset: entry.offset,
                length: entry.length,
                orig_length: entry.orig_length,
            };

            let decompressed: anyhow::Result<Vec<u8>> = match &entry.entry_type {
                EntryType::Full => {
                    if let Some(sb) = superblock {
                        let data = &sb[loc.offset as usize..][..loc.length as usize];
                        zstd::bulk::decompress(data, loc.orig_length as usize).map_err(|e| anyhow::anyhow!("{e}"))
                    } else {
                        pack::read_chunk(pack_data, &loc).map_err(|e| anyhow::anyhow!("{e}"))
                    }
                }
                EntryType::FullRaw => {
                    if let Some(sb) = superblock {
                        Ok(sb[loc.offset as usize..][..loc.length as usize].to_vec())
                    } else {
                        pack::read_raw_chunk(pack_data, &loc).map_err(|e| anyhow::anyhow!("{e}"))
                    }
                }
                EntryType::Delta { base_hash } => match read_chunk_by_hash(&all_packs, base_hash) {
                    Ok(base_chunk) => {
                        pack::read_delta_chunk(pack_data, &loc, &base_chunk).map_err(|e| anyhow::anyhow!("{e}"))
                    }
                    Err(e) => {
                        eprintln!("    ✗ Chunk at offset {}: base chunk lookup failed: {e}", entry.offset);
                        failed += 1;
                        continue;
                    }
                },
            };

            match decompressed {
                Ok(decompressed) => {
                    let actual_hash = Hash::from_blake3(blake3::hash(&decompressed));
                    if actual_hash != entry.hash {
                        eprintln!("    ✗ Chunk hash mismatch at offset {}", entry.offset);
                        failed += 1;
                    }
                }
                Err(e) => {
                    eprintln!("    ✗ Chunk at offset {} decompression failed: {e}", entry.offset);
                    failed += 1;
                }
            }
        }
    }

    println!("\nVerification complete:");
    println!("  Verified: {} packs", verified);
    if failed > 0 {
        println!("  FAILED:   {} checks", failed);
        anyhow::bail!("Verification found {failed} errors");
    } else {
        println!("  All checks passed ✓");
    }

    Ok(())
}
