use anyhow::{Context, Result};
use packt_lib::store::Store;
use std::path::Path;

pub fn run_restore(source: &str, destination: &Path, file_name: Option<&str>) -> Result<()> {
    let config = Store::config_from_uri(source).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    store.restore(destination, file_name).context("Failed to restore")?;

    Ok(())
}
