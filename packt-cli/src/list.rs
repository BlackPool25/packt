use anyhow::{Context, Result};
use packt_lib::store::Store;

pub fn run_list(store_uri: &str) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let files = store.list_files().context("Failed to list files")?;

    println!("Store: {store_uri} ({} files)", files.len());
    println!();

    if files.is_empty() {
        println!("  No files found.");
        return Ok(());
    }

    for f in &files {
        let mtime = if f.modified.is_empty() || f.modified == "0" {
            String::new()
        } else {
            format!(" (modified: {})", f.modified)
        };
        println!("  {}{}", f.name, mtime);
        println!("    size: {} bytes, chunks: {}", f.size, f.chunk_count);
    }

    Ok(())
}
