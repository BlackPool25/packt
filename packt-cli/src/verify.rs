use anyhow::Result;
use std::path::Path;

pub fn run_verify(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let packs_dir = path.join("packs");
    if !packs_dir.exists() {
        anyhow::bail!("No packs directory found at: {}", packs_dir.display());
    }

    let pack_files: Vec<_> = std::fs::read_dir(&packs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "pack"))
        .collect();

    if pack_files.is_empty() {
        println!("No pack files to verify.");
        return Ok(());
    }

    let mut verified = 0u64;
    let mut failed = 0u64;

    for entry in &pack_files {
        let data = std::fs::read(entry.path())?;
        match packt_lib::store::pack::read_pack(&data) {
            Ok((entries, checksum)) => {
                eprintln!(
                    "  ✓ {} ({} chunks, checksum: {})",
                    entry.file_name().to_string_lossy(),
                    entries.len(),
                    hex::encode(checksum)
                );
                verified += 1;

                // Verify each chunk decompresses correctly
                for entry in &entries {
                    let loc = packt_lib::types::PackLocation {
                        pack_id: 0,
                        offset: entry.offset,
                        length: entry.length,
                        orig_length: entry.orig_length,
                    };
                    match packt_lib::store::pack::read_chunk(&data, &loc) {
                        Ok(decompressed) => {
                            // Verify hash matches
                            let actual_hash = packt_lib::types::Hash::from_blake3(blake3::hash(&decompressed));
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
            Err(e) => {
                eprintln!("  ✗ {} verification failed: {e}", entry.file_name().to_string_lossy());
                failed += 1;
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
