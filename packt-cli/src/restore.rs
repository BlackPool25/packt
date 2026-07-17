use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use packt_lib::store::Store;
use serde::Serialize;
use std::path::Path;

use crate::GlobalOpts;

#[derive(Serialize)]
struct Output {
    status: String,
    files_restored: usize,
}

pub fn run_restore(source: &str, destination: &Path, file_name: Option<&str>, opts: &GlobalOpts) -> Result<()> {
    let config = Store::config_from_uri(source).context("Failed to parse store URI")?;
    let store = Store::open(config).context("Failed to open store")?;

    let pb = if !opts.quiet && !opts.json {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::with_template("{spinner:.green} {msg}").unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        let msg = match file_name {
            Some(f) => format!("Restoring {f} ..."),
            None => "Restoring all files ...".into(),
        };
        bar.set_message(msg);
        bar.enable_steady_tick(std::time::Duration::from_millis(100));
        Some(bar)
    } else {
        None
    };

    store.restore(destination, file_name).context("Failed to restore")?;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    if opts.json {
        let out = Output {
            status: "done".into(),
            files_restored: 1,
        };
        println!("{}", serde_json::to_string(&out)?);
    } else if !opts.quiet {
        match file_name {
            Some(f) => println!("Restored: {f}"),
            None => println!("Restore complete"),
        }
    }

    Ok(())
}
