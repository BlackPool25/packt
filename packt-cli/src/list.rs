use anyhow::{Context, Result};
use packt_lib::store::Store;
use serde::Serialize;

use crate::GlobalOpts;

#[derive(Serialize)]
struct FileEntry {
    name: String,
    size: u64,
    chunk_count: usize,
}

#[derive(Serialize)]
struct Output {
    store: String,
    file_count: usize,
    files: Vec<FileEntry>,
}

pub fn run_list(store_uri: &str, opts: &GlobalOpts) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;
    let files = store.list_files().context("Failed to list files")?;

    if opts.json {
        let out = Output {
            store: store_uri.to_string(),
            file_count: files.len(),
            files: files
                .iter()
                .map(|f| FileEntry {
                    name: f.name.clone(),
                    size: f.size,
                    chunk_count: f.chunk_count,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string(&out)?);
    } else {
        if opts.quiet && !files.is_empty() {
            for f in &files {
                println!("{}", f.name);
            }
            return Ok(());
        }
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
    }

    Ok(())
}
