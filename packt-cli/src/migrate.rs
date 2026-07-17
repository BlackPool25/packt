use anyhow::{Context, Result};
use packt_lib::store::{BackupOpts, Store};
use serde::Serialize;

use crate::GlobalOpts;

#[derive(Serialize)]
struct FileProgress {
    name: String,
    chunks: u64,
}

#[derive(Serialize)]
struct Output {
    source: String,
    destination: String,
    total_files: usize,
    total_bytes: u64,
    files: Vec<FileProgress>,
}

pub fn run_migrate(source: &str, destination: &str, opts: &GlobalOpts) -> Result<()> {
    let src_config = Store::config_from_uri(source).context("Failed to parse source URI")?;
    let dst_config = Store::config_from_uri(destination).context("Failed to parse destination URI")?;

    let src = Store::open(src_config).context("Failed to open source store")?;
    let dst = Store::open(dst_config).context("Failed to open destination store")?;

    let files = src.list_files().context("Failed to list source files")?;

    if files.is_empty() {
        if !opts.quiet {
            println!("No files to migrate.");
        }
        return Ok(());
    }

    let tmp = tempfile::TempDir::new().context("Failed to create temp directory")?;
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;
    let mut progress: Vec<FileProgress> = Vec::new();

    for file in &files {
        if !opts.quiet && !opts.json {
            eprintln!("  Migrating: {} ...", file.name);
        }

        src.restore(tmp.path(), Some(&file.name))
            .with_context(|| format!("Failed to restore {}", file.name))?;

        let restored_path = tmp.path().join(&file.name);
        if !restored_path.exists() {
            anyhow::bail!("Restored file not found: {}", restored_path.display());
        }

        let stats = dst
            .backup(
                &restored_path,
                &BackupOpts {
                    force: true,
                    ..Default::default()
                },
            )
            .with_context(|| format!("Failed to backup {}", file.name))?;

        total_files += 1;
        total_bytes += stats.source_size;
        progress.push(FileProgress {
            name: file.name.clone(),
            chunks: stats.total_chunks,
        });

        std::fs::remove_file(&restored_path).ok();
    }

    if opts.json {
        let out = Output {
            source: source.to_string(),
            destination: destination.to_string(),
            total_files,
            total_bytes,
            files: progress,
        };
        println!("{}", serde_json::to_string(&out)?);
    } else if !opts.quiet {
        println!("Migration complete: {total_files} files, {total_bytes} bytes");
        println!("  Source:      {source}");
        println!("  Destination: {destination}");
    }

    Ok(())
}
