use std::path::Path;
use anyhow::{Context, Result};
use packt_lib::store::ContentStore;
use std::os::unix::fs::PermissionsExt;

use crate::backup::ManifestEntry;

/// Parse a manifest as either [`ManifestEntry`] (new) or [`Vec<String>`] (legacy).
fn parse_manifest(bytes: &[u8]) -> Result<ManifestEntry> {
    // Try new format first.
    if let Ok(entry) = serde_json::from_slice::<ManifestEntry>(bytes) {
        return Ok(entry);
    }
    // Fall back to legacy Vec<String> format.
    let hashes: Vec<String> = serde_json::from_slice(bytes)?;
    Ok(ManifestEntry {
        path: String::new(),
        size: 0,
        modified: String::new(),
        permissions: 0,
        chunk_hashes: hashes,
    })
}

/// Restore modification time and permissions on the output file.
fn restore_metadata(path: &Path, entry: &ManifestEntry) {
    // Restore mtime.
    if let Ok(secs) = entry.modified.parse::<u64>() {
        let ft = filetime::FileTime::from_unix_time(secs as i64, 0);
        filetime::set_file_mtime(path, ft).ok();
    }
    // Restore permissions.
    if entry.permissions != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(entry.permissions)).ok();
    }
}

pub fn run_restore(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Store path does not exist: {}", source.display());
    }

    let store = packt_lib::store::local::LocalStore::open(source)
        .context("Failed to open store")?;

    std::fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create destination: {}", destination.display()))?;

    let manifests_dir = source.join("manifests");
    if !manifests_dir.exists() {
        anyhow::bail!("No manifests directory found. No files have been backed up with manifest tracking.");
    }

    let mut restored_count = 0u64;
    let mut total_bytes = 0u64;

    for entry in std::fs::read_dir(&manifests_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "manifest") {
            let manifest_bytes = std::fs::read(&path)?;
            let manifest_entry = parse_manifest(&manifest_bytes)
                .with_context(|| format!("Failed to parse manifest: {}", path.display()))?;

            // Determine output path: use stored path if available, else manifest file stem.
            let out_path = if manifest_entry.path.is_empty() {
                destination.join(path.file_stem().unwrap())
            } else {
                destination.join(&manifest_entry.path)
            };

            // Reconstruct the file from chunk hashes.
            let mut file_data = Vec::new();
            for hash_hex in &manifest_entry.chunk_hashes {
                let hash = packt_lib::types::Hash::from_hex(hash_hex)
                    .map_err(|e| anyhow::anyhow!("Invalid hash in manifest: {e}"))?;
                let chunk_data = store.get(&hash)
                    .with_context(|| format!("Failed to read chunk {hash_hex}"))?;
                file_data.extend_from_slice(&chunk_data);
            }

            // Write reconstructed file.
            std::fs::write(&out_path, &file_data)
                .with_context(|| format!("Failed to write output: {}", out_path.display()))?;

            // Restore metadata.
            restore_metadata(&out_path, &manifest_entry);

            restored_count += 1;
            total_bytes += file_data.len() as u64;
            eprintln!("  ✓ Restored {} ({} chunks, {} bytes)",
                manifest_entry.path, manifest_entry.chunk_hashes.len(), file_data.len());
        }
    }

    if restored_count == 0 {
        eprintln!("No manifests found in store.");
        return Ok(());
    }

    println!("Restore complete: {restored_count} files, {total_bytes} bytes to {}",
        destination.display());

    // Verify reconstruction
    for entry in std::fs::read_dir(&manifests_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "manifest") {
            let manifest_bytes = std::fs::read(&path)?;
            let manifest_entry = parse_manifest(&manifest_bytes)?;
            let chunk_hashes = &manifest_entry.chunk_hashes;

            let out_path = if manifest_entry.path.is_empty() {
                destination.join(path.file_stem().unwrap())
            } else {
                destination.join(&manifest_entry.path)
            };

            if out_path.exists() {
                let data = std::fs::read(&out_path)?;
                // Verify each chunk sequentially
                let mut offset = 0u64;
                for hash_hex in chunk_hashes {
                    let hash = packt_lib::types::Hash::from_hex(hash_hex).unwrap();
                    let chunk_data = store.get(&hash)?;
                    let expected = &data[offset as usize..offset as usize + chunk_data.len()];
                    if chunk_data != expected {
                        anyhow::bail!(
                            "INTEGRITY FAILURE: chunk {hash_hex} at offset {offset} does not match restored file!"
                        );
                    }
                    offset += chunk_data.len() as u64;
                }
                eprintln!("  ✓ {} integrity verified", manifest_entry.path);
            }
        }
    }

    Ok(())
}
