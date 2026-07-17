use anyhow::{Context, Result};
use packt_lib::store::Store;
use serde::Serialize;

use crate::GlobalOpts;

#[derive(Serialize)]
struct Output {
    store: String,
    file_count: usize,
    total_source_bytes: u64,
    total_chunks: usize,
}

pub fn run_info(store_uri: &str, opts: &GlobalOpts) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let info = store.info().context("Failed to get store info")?;

    if opts.json {
        let out = Output {
            store: store_uri.to_string(),
            file_count: info.file_count,
            total_source_bytes: info.total_source_bytes,
            total_chunks: info.total_chunks,
        };
        println!("{}", serde_json::to_string(&out)?);
    } else if !opts.quiet {
        println!("Store: {store_uri}");
        println!("  Files:          {}", info.file_count);
        println!(
            "  Total size:     {} bytes ({:.2} MB)",
            info.total_source_bytes,
            info.total_source_bytes as f64 / 1_048_576.0
        );
        println!("  Total chunks:   {}", info.total_chunks);
    }

    Ok(())
}
