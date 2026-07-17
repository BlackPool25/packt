use anyhow::{Context, Result};
use packt_lib::store::ContentStore;
use std::path::Path;

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
    // Restore permissions (Unix only).
    set_unix_permissions(path, entry.permissions);
}

/// Set Unix file permissions. No-op on non-Unix platforms.
#[cfg(unix)]
fn set_unix_permissions(path: &Path, mode: u32) {
    if mode != 0 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).ok();
    }
}

/// Set Unix file permissions. No-op on non-Unix platforms.
#[cfg(not(unix))]
fn set_unix_permissions(_path: &Path, _mode: u32) {}

pub fn run_restore(source: &Path, destination: &Path, file_name: Option<&str>) -> Result<()> {
    if !source.exists() {
        anyhow::bail!("Store path does not exist: {}", source.display());
    }

    let store = packt_lib::store::local::LocalStore::open(source).context("Failed to open store")?;

    std::fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create destination: {}", destination.display()))?;

    let manifests_dir = source.join("manifests");
    if !manifests_dir.exists() {
        anyhow::bail!("No manifests directory found. No files have been backed up with manifest tracking.");
    }

    let mut manifest_files: Vec<_> = std::fs::read_dir(&manifests_dir)?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "manifest"))
        .collect();
    manifest_files.sort_by_key(|e| e.file_name());

    // Filter by file name if specified.
    let matching: Vec<_> = if let Some(name) = file_name {
        manifest_files
            .into_iter()
            .filter(|e| {
                let p = e.path();
                let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                if stem == name {
                    return true;
                }
                if let Ok(bytes) = std::fs::read(&p) {
                    if let Ok(m) = serde_json::from_slice::<ManifestEntry>(&bytes) {
                        return m.path == name;
                    }
                }
                false
            })
            .collect()
    } else {
        manifest_files
    };

    if matching.is_empty() {
        if let Some(name) = file_name {
            anyhow::bail!("No manifest found for file: {name:?}");
        }
        eprintln!("No manifests found in store.");
        return Ok(());
    }

    let mut restored_count = 0u64;
    let mut total_bytes = 0u64;

    for entry in &matching {
        let path = entry.path();
        let manifest_bytes = std::fs::read(&path)?;
        let manifest_entry = parse_manifest(&manifest_bytes)
            .with_context(|| format!("Failed to parse manifest: {}", path.display()))?;

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
            let chunk_data = store
                .get(&hash)
                .with_context(|| format!("Failed to read chunk {hash_hex}"))?;
            file_data.extend_from_slice(&chunk_data);
        }

        std::fs::write(&out_path, &file_data)
            .with_context(|| format!("Failed to write output: {}", out_path.display()))?;

        restore_metadata(&out_path, &manifest_entry);

        restored_count += 1;
        total_bytes += file_data.len() as u64;
        eprintln!(
            "  Restored {} ({} chunks, {} bytes)",
            if manifest_entry.path.is_empty() {
                path.file_stem().unwrap().to_string_lossy().to_string()
            } else {
                manifest_entry.path.clone()
            },
            manifest_entry.chunk_hashes.len(),
            file_data.len()
        );
    }

    if restored_count == 0 {
        eprintln!("No files restored.");
        return Ok(());
    }

    println!(
        "Restore complete: {restored_count} files, {total_bytes} bytes to {}",
        destination.display()
    );

    // Verify reconstruction for restored files.
    for entry in &matching {
        let path = entry.path();
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
            eprintln!("  Verified integrity of restored file");
        }
    }

    Ok(())
}
