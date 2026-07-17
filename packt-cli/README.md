# packt-cli

[![Crates.io][crates-badge]][crates-url]
[![License][license-badge]][license-url]

[crates-badge]: https://img.shields.io/crates/v/packt-cli.svg
[crates-url]: https://crates.io/crates/packt-cli
[license-badge]: https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg
[license-url]: https://github.com/BlackPool25/packt#license

CLI for packt-lib -- content-defined chunking with exact dedup, near-duplicate detection, and delta compression.

## Install

```bash
cargo install packt-cli
```

## Usage

```bash
# Backup a file (incremental -- skips if unchanged since last backup)
packt backup ./myfile.big ./backup-store/

# Force re-backup even if unchanged
packt backup --force ./myfile.big ./backup-store/

# Enable near-duplicate detection and delta compression
packt backup --similarity-threshold 0.7 ./myfile.big ./backup-store/

# List all backed up files
packt list ./backup-store/

# Show store statistics
packt info ./backup-store/

# Verify all pack integrity
packt verify ./backup-store/

# Restore all files
packt restore ./backup-store/ ./restored/

# Restore a single file by name
packt restore ./backup-store/ ./restored/ myfile.big
```

### Options

```
--chunk-size <BYTES>               Average chunk size (default: 32768).
--similarity-threshold <0-1>       Near-dup detection threshold (default: 0.7, 0 = disable).
--force                            Force re-backup even if file unchanged.
```

## Library

For embedding packt in your Rust project, use `packt-lib` instead.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
