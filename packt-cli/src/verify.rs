use anyhow::{Context, Result};
use packt_lib::store::Store;
use serde::Serialize;

use crate::GlobalOpts;

#[derive(Serialize)]
struct Output {
    store: String,
    files_checked: usize,
    chunks_checked: usize,
    ok: bool,
    errors: Vec<String>,
}

pub fn run_verify(store_uri: &str, opts: &GlobalOpts) -> Result<()> {
    let config = Store::config_from_uri(store_uri).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let report = store.verify(None).context("Verification failed")?;

    if opts.json {
        let out = Output {
            store: store_uri.to_string(),
            files_checked: report.files_checked,
            chunks_checked: report.chunks_checked,
            ok: report.ok,
            errors: report.errors.clone(),
        };
        println!("{}", serde_json::to_string(&out)?);
    } else if !opts.quiet {
        println!("\nVerification complete:");
        println!("  Files checked:  {}", report.files_checked);
        println!("  Chunks checked: {}", report.chunks_checked);
        if report.ok {
            println!("  All checks passed \u{2713}");
        } else {
            for err in &report.errors {
                eprintln!("  {err}");
            }
        }
    }
    if !report.ok && !opts.json {
        anyhow::bail!("Verification found {} errors", report.errors.len());
    }

    Ok(())
}
