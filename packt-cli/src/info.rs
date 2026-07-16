use std::path::Path;
use anyhow::Result;

pub fn run_info(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Path does not exist: {}", path.display());
    }

    let packs_dir = path.join("packs");
    if !packs_dir.exists() {
        println!("Store directory exists but contains no packs yet.");
        println!("Path: {}", path.display());
        return Ok(());
    }

    // Collect pack files in single pass
    let pack_files: Vec<_> = std::fs::read_dir(&packs_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "pack"))
        .collect();
    let pack_count = pack_files.len();

    let total_pack_size: u64 = pack_files.iter()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    println!("Store: {}", path.display());
    println!("  Packs:        {} files", pack_count);
    println!("  Pack size:     {} bytes ({:.2} MB)", total_pack_size, total_pack_size as f64 / 1_048_576.0);

    Ok(())
}
