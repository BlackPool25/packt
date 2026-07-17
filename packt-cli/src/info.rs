use anyhow::{Context, Result};
use packt_lib::store::Store;

pub fn run_info(store_uri: &str) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let info = store.info().context("Failed to get store info")?;

    println!("Store: {store_uri}");
    println!("  Files:          {}", info.file_count);
    println!(
        "  Total size:     {} bytes ({:.2} MB)",
        info.total_source_bytes,
        info.total_source_bytes as f64 / 1_048_576.0
    );
    println!("  Total chunks:   {}", info.total_chunks);

    Ok(())
}
