use anyhow::{Context, Result};
use packt_lib::store::{BackupOpts, Store};

pub fn run_migrate(source: &str, destination: &str) -> Result<()> {
    let src_config = Store::config_from_uri(source).context("Failed to parse source URI")?;
    let dst_config = Store::config_from_uri(destination).context("Failed to parse destination URI")?;

    let src = Store::open(src_config).context("Failed to open source store")?;
    let dst = Store::open(dst_config).context("Failed to open destination store")?;

    let files = src.list_files().context("Failed to list source files")?;

    if files.is_empty() {
        println!("No files to migrate.");
        return Ok(());
    }

    // Use a temp directory for intermediate file reconstruction
    let tmp = tempfile::TempDir::new().context("Failed to create temp directory")?;
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;

    for file in &files {
        eprintln!("  Migrating: {} ...", file.name);

        // Restore from source
        src.restore(tmp.path(), Some(&file.name))
            .with_context(|| format!("Failed to restore {}", file.name))?;

        let restored_path = tmp.path().join(&file.name);
        if !restored_path.exists() {
            anyhow::bail!("Restored file not found: {}", restored_path.display());
        }

        // Backup to destination
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

        // Clean up temp file
        std::fs::remove_file(&restored_path).ok();

        eprintln!("    Done ({} chunks)", stats.total_chunks);
    }

    println!("Migration complete: {total_files} files, {total_bytes} bytes");
    println!("  Source:      {source}");
    println!("  Destination: {destination}");

    Ok(())
}
