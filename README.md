# Sieve

**Content-defined chunking with exact dedup for binary data.**

A Rust library and CLI for splitting binary data into content-defined chunks (FastCDC v2020), deduplicating identical chunks via BLAKE3 content-addressing, and storing them in an integrity-verified pack format.

## Project Status

Phase 1 implementation — CDC + exact dedup (production hardening complete).

## Architecture

```
Source file → FastCDC chunking → BLAKE3 hashing → Exact dedup → Pack storage
```

1. **Chunking**: FastCDC v2020 splits files at content-defined boundaries (default 32KB average)
2. **Hashing**: BLAKE3 identifies each chunk uniquely (32-byte content hash)
3. **Dedup**: Concurrent index with Bloom filter checks if a chunk has been stored before; index pre-populated from existing packs on open
4. **Storage**: New chunks are zstd-compressed and stored in pack files with BLAKE3 integrity checksums

## Quick Start

```bash
# Build
cargo build --release

# Backup a file
./target/release/sieve backup ./myfile.big ./backup-store/

# Check store info
./target/release/sieve info ./backup-store/

# Verify pack integrity (checks every chunk)
./target/release/sieve verify ./backup-store/

# Restore files
./target/release/sieve restore ./backup-store/ ./restored/
```

## CLI

```
sieve backup <source> <destination>     Create deduplicated backup
sieve restore <source> <dest-dir>       Restore from backup
sieve info <path>                       Show store statistics
sieve verify <path>                     Verify pack integrity
sieve benchmark <corpus>                Run performance benchmarks
```

Backups store file metadata (path, size, modification time, permissions) alongside chunk hashes. Restore preserves all metadata.

## Library Usage

```rust
use std::sync::Arc;
use sieve_lib::chunking::fastcdc::FastCdcChunker;
use sieve_lib::hash::blake3_hasher::Blake3Hasher;
use sieve_lib::index::hashindex::HashIndex;
use sieve_lib::pipeline::{BackupPipeline, PipelineConfig};
use sieve_lib::store::local::LocalStore;
use sieve_lib::types::ChunkConfig;

// Setup
let store = Arc::new(LocalStore::open("./backup-store")?);
let index = Arc::new(HashIndex::new(1_000_000));
store.populate_index(&index)?; // Load existing chunks

let chunker = Arc::new(FastCdcChunker::new(ChunkConfig::default_32k()));
let hasher = Arc::new(Blake3Hasher::new());

// Pipeline
let pipeline = BackupPipeline::new(
    PipelineConfig::default(), chunker, hasher, store, index,
);
let stats = pipeline.backup_file("./myfile.big")?;
println!("Dedup ratio: {:.2}x", stats.dedup_ratio());
```

## Development

```bash
# Build
cargo build

# Test (unit + integration)
cargo test

# Benchmarks
cargo bench

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt --check

# Security audit
cargo audit
```

## Project Structure

```
sieve/
├── sieve-lib/              # Library crate (the product)
│   └── src/
│       ├── chunking/       # FastCDC v2020 chunking
│       ├── hash/           # BLAKE3 content hashing
│       ├── store/          # Pack format + local filesystem backend
│       ├── index/          # Concurrent dedup index with Bloom filter
│       ├── pipeline/       # Pipeline orchestrator (streaming)
│       └── types.rs        # Core types: Chunk, Hash, PackLocation
├── sieve-cli/              # CLI binary (the demo/dogfood)
└── sieve-lib/tests/        # Integration tests
```

## Performance

Real-world validation on Docker image layers:
- Ubuntu 22.04→24.04 (2 versions): **3.8x** dedup ratio
- Cross-image (9 Docker images): **12.75x** (tied with restic)
- Backup speed: **2.2x faster** than restic on same data
- Streaming: processes files of any size with ~8 MB peak memory

## License

MIT OR Apache-2.0
