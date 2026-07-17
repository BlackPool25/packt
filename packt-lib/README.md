# packt-lib

[![Crates.io][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]
[![License][license-badge]][license-url]

[crates-badge]: https://img.shields.io/crates/v/packt-lib.svg
[crates-url]: https://crates.io/crates/packt-lib
[docs-badge]: https://img.shields.io/docsrs/packt-lib
[docs-url]: https://docs.rs/packt-lib
[license-badge]: https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg
[license-url]: https://github.com/BlackPool25/packt#license

Content-defined chunking with exact dedup, near-duplicate detection, zstd delta compression, and cloud storage support.

## Quick Start

### Local Store (no optional deps)

```toml
[dependencies]
packt-lib = "0.6"
```

```rust
use packt_lib::store::{Store, StoreConfig, BackupOpts};
use packt_lib::types::Hash;

// Open a local store
let store = Store::open(StoreConfig::Local {
    path: "./backup-store".into(),
})?;

// Backup a file (dedup + compress + manifest)
let stats = store.backup("myfile.big".as_ref(), &BackupOpts::default())?;
println!("Ratio: {:.2}x", stats.dedup_ratio());

// List backed-up files
for file in store.list_files()? {
    println!("  {} ({} chunks)", file.name, file.chunk_count);
}

// Restore a file
store.restore("./restored".as_ref(), Some("myfile.big"))?;
```

### S3 / GCS Store (enable `cloud` feature)

```toml
[dependencies]
packt-lib = { version = "0.6", features = ["cloud"] }
```

```rust
use packt_lib::store::{Store, StoreConfig};

// S3
let store = Store::open(StoreConfig::S3 {
    bucket: "my-backups".into(),
    region: Some("us-east-1".into()),
    endpoint: None,
    access_key_id: None,
    secret_access_key: None,
    cache_dir: Some("./cache".into()),
})?;

// GCS
let store = Store::open(StoreConfig::GCS {
    bucket: "my-backups".into(),
    prefix: Some("servers/".into()),
    cache_dir: Some("./cache".into()),
})?;

// URI shortcut (available in CLI, also works in library)
let config = Store::config_from_uri("s3://my-backups/servers/?region=us-east-1")?;
let store = Store::open(config)?;
```

## Features

- **Content-Defined Chunking** -- FastCDC v2020 with configurable chunk sizes (default 32 KB).
- **Exact Dedup** -- BLAKE3 content addressing with concurrent DashMap index and Bloom filter.
- **Near-Duplicate Detection** -- Palantir 3-tier hierarchical super-features (~0 ms overhead).
- **Delta Compression** -- zstd dictionary mode for similar chunks with automatic fallback.
- **Cloud Storage** -- S3 and GCS via OpenDAL (`cloud` feature). Optional local LRU cache.
- **High-Level Store API** -- `Store::open/backup/restore/list/info/verify/delete/hasFile`.
- **Integrity Verification** -- BLAKE3 checksums on every chunk, verified on read.
- **Cross-Session Dedup** -- Similarity signatures persisted in pack format, index rebuilt on open.

## Performance

| Workload | packt 0.5 | restic 0.19 | Advantage |
|---|---|---|---|
| Docker cross-image (5 images, 395 MB) | 4.0x (90 MB) | 2.7x (144 MB) | 38% better |
| VM snapshots (4 versions, 200 MB) | 8.1x (25 MB) | 3.2x (62 MB) | 60% better |
| Throughput (Docker layers) | 118 MB/s | ~100 MB/s | 18% faster |

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
