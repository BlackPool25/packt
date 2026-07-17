use anyhow::Result;
use std::path::Path;

use crate::backup::ManifestEntry;

pub fn run_list(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Store path does not exist: {}", path.display());
    }

    let manifests_dir = path.join("manifests");
    if !manifests_dir.exists() {
        println!("Store: {} (0 files)", path.display());
        return Ok(());
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&manifests_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "manifest") {
            let bytes = std::fs::read(&path)?;
            if let Ok(manifest) = serde_json::from_slice::<ManifestEntry>(&bytes) {
                let size = manifest.chunk_hashes.len();
                entries.push((manifest.path, manifest.size, manifest.modified.clone(), size));
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    println!("Store: {} ({} files)", path.display(), entries.len());
    println!();
    if entries.is_empty() {
        println!("  No files found.");
        return Ok(());
    }
    for (name, size, modified, chunks) in &entries {
        let mtime = if modified.is_empty() || modified == "0" {
            String::new()
        } else {
            format!(" (modified: {})", modified)
        };
        println!("  {}{}", name, mtime);
        println!("    size: {} bytes, chunks: {}", size, chunks);
    }

    Ok(())
}
