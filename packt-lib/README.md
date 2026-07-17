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

Content-defined chunking with exact dedup, near-duplicate detection, and zstd delta compression for binary data.

## Quick Start

```toml
[dependencies]
packt-lib = "0.5"
```

```rust
use std::sync::Arc;
use packt_lib::chunking::fastcdc::FastCdcChunker;
use packt_lib::hash::blake3_hasher::Blake3Hasher;
use packt_lib::index::hashindex::HashIndex;
use packt_lib::pipeline::{BackupPipeline, PipelineConfig};
use packt_lib::store::local::LocalStore;
use packt_lib::types::ChunkConfig;

// Open a dedup store
let store = Arc::new(LocalStore::open("./backup-store")?);
let index = Arc::new(HashIndex::new(1_000_000));
store.populate_index(&index)?;

let pipeline = BackupPipeline::new(
    PipelineConfig::default(),
    Arc::new(FastCdcChunker::new(ChunkConfig::default_32k())),
    Arc::new(Blake3Hasher::new()),
    store,
    index,
);
let stats = pipeline.backup_file("./myfile.big")?;
println!("Dedup ratio: {:.2}x", stats.dedup_ratio());
```

## Features

- **Content-Defined Chunking** -- FastCDC v2020 with configurable chunk sizes (default 32 KB).
- **Exact Dedup** -- BLAKE3 content addressing with concurrent DashMap index and Bloom filter.
- **Near-Duplicate Detection** -- Palantir 3-tier hierarchical super-features (~0 ms overhead).
- **Delta Compression** -- zstd dictionary mode for similar chunks with automatic fallback.
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
