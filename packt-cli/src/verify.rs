use anyhow::{Context, Result};
use packt_lib::store::Store;

pub fn run_verify(store_uri: &str) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let report = store.verify(None).context("Verification failed")?;

    println!("\nVerification complete:");
    println!("  Files checked:  {}", report.files_checked);
    println!("  Chunks checked: {}", report.chunks_checked);
    if report.ok {
        println!("  All checks passed ✓");
    } else {
        for err in &report.errors {
            eprintln!("  {err}");
        }
        println!("  FAILED: {} errors", report.errors.len());
        anyhow::bail!("Verification found {} errors", report.errors.len());
    }

    Ok(())
}
