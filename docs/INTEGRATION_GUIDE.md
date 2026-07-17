# Packt Integration Guide

This guide covers using packt as a library for deduplication storage in your own applications.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Basic Library Usage](#basic-library-usage)
3. [Cloud Storage](#cloud-storage)
4. [Caching](#caching)
5. [Migration](#migration)
6. [Feature Flags](#feature-flags)
7. [Docker/CI Cache Pattern](#dockerci-cache-pattern)
8. [Production Considerations](#production-considerations)

## Architecture Overview

Packt has two layers:

- **Low-level traits**: `ContentStore`, `DedupIndex`, `Chunker`, `ContentHasher` — for custom integrations
- **High-level Store API**: `Store::open/backup/restore/list/info/verify/delete/hasFile` — drop-in dedup storage

```
┌─────────────────────────────────────────────────────┐
│                    Your Application                  │
├─────────────────────────────────────────────────────┤
│                  Store API (facade)                  │
├──────────────┬──────────────────┬───────────────────┤
│  LocalStore  │   CloudStore     │  (future backends) │
│  (std::fs)   │ (OpenDAL S3/GCS) │                    │
├──────────────┴──────────────────┴───────────────────┤
│                   PACKv3 Format                      │
│          (zstd compression + BLAKE3 integrity)       │
│    FastCDC chunking + Palantir similarity + delta    │
└─────────────────────────────────────────────────────┘
```

## Basic Library Usage

### Local Store

```rust
use packt_lib::store::{Store, StoreConfig, BackupOpts};

let store = Store::open(StoreConfig::Local {
    path: "./my-backup-store".into(),
})?;

// Backup a file
let stats = store.backup("important.dat".as_ref(), &BackupOpts::default())?;
println!("Dedup ratio: {:.2}x", stats.dedup_ratio());
println!("Chunks: {} unique / {} total", stats.unique_chunks, stats.total_chunks);

// List backed-up files
let files = store.list_files()?;
for f in &files {
    println!("{} ({} bytes, {} chunks)", f.name, f.size, f.chunk_count);
}

// Restore a single file
store.restore("./output".as_ref(), Some("important.dat"))?;

// Verify integrity
let report = store.verify(None)?;
assert!(report.ok, "Verification failed: {:?}", report.errors);

// Get store info
let info = store.info()?;
println!("{} files, {} total bytes", info.file_count, info.total_source_bytes);

// Delete a file (removes manifest only — no GC yet)
store.delete_file("old-file.dat")?;
```

### Backup Options

```rust
let opts = BackupOpts {
    chunk_size: 65536,           // average chunk size (default: 32768)
    similarity_threshold: 0.7,   // 0.0 = disable, 0.7 = default
    force: false,                // true = skip mtime check
};
```

### Incremental Backups

If the source file's size and mtime match the stored manifest, `Store::backup()`
skips it entirely. Use `force: true` to override.

```rust
// First backup — processes the file
store.backup("data.bin".as_ref(), &BackupOpts::default())?;

// Second backup with unchanged file — skipped
store.backup("data.bin".as_ref(), &BackupOpts::default())?;

// Force re-backup
store.backup("data.bin".as_ref(), &BackupOpts { force: true, ..Default::default() })?;
```

## Cloud Storage

Enable the `cloud` feature:

```toml
[dependencies]
packt-lib = { version = "0.6", features = ["cloud"] }
```

### S3

Configuration via `StoreConfig::S3`:

```rust
use packt_lib::store::{Store, StoreConfig};

let store = Store::open(StoreConfig::S3 {
    bucket: "my-backups".into(),
    region: Some("us-east-1".into()),
    endpoint: None,                    // default: https://s3.amazonaws.com
    access_key_id: Some("AKIA...".into()),  // optional — falls back to env
    secret_access_key: Some("...".into()),  // optional — falls back to env
    cache_dir: Some("./cache".into()),      // optional local LRU cache
})?;
```

Credentials are resolved in this order:
1. Explicit `access_key_id` / `secret_access_key` in config
2. AWS environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
3. AWS credentials file (`~/.aws/credentials`)
4. IAM instance profile (EC2, ECS, etc.)

For MinIO or S3-compatible storage:

```rust
let store = Store::open(StoreConfig::S3 {
    bucket: "test-bucket".into(),
    region: Some("auto".into()),
    endpoint: Some("http://localhost:9000".into()),
    access_key_id: Some("minioadmin".into()),
    secret_access_key: Some("minioadmin".into()),
    cache_dir: None,
})?;
```

### GCS

```rust
let store = Store::open(StoreConfig::GCS {
    bucket: "my-backups".into(),
    prefix: Some("servers/production/".into()),
    cache_dir: Some("./cache".into()),
})?;
```

GCS credentials use Google Application Default Credentials:
- `GOOGLE_APPLICATION_CREDENTIALS` environment variable
- GCP metadata server (Compute Engine, GKE, etc.)

### Cloud Store Layout

```
s3://my-bucket/
├── packs/
│   ├── 00000000.pack        # Pack files (16MB each, PACKv3 format)
│   ├── 00000001.pack
│   └── _meta.index          # Global chunk index (for fast reopen)
└── manifests/
    ├── data.bin.manifest    # File manifests (JSON)
    └── config.tar.manifest
```

All paths are flat (no subdirectories within packs/). The `_meta.index` is
maintained automatically and updated on each flush.

## Caching

### Local Disk Cache for Cloud Stores

When `cache_dir` is set, the cloud store caches downloaded pack files
locally. This avoids re-downloading packs for repeated reads.

```rust
// Enable cache with 128-pack LRU (~2GB at 16MB/pack)
let store = Store::open(StoreConfig::S3 {
    bucket: "my-bucket".into(),
    region: Some("us-east-1".into()),
    cache_dir: Some("/var/cache/packt".into()),
    ..Default::default()
})?;
```

Cache behavior:
- Pack files are stored at `{cache_dir}/packs/{pack_id:08}.pack`
- LRU eviction (default: 128 entries)
- Writing a pack also writes to cache (warm cache for future reads)
- No cache = every `get()` downloads the full 16MB pack from cloud
- Cache is safe to delete at any time (will be re-downloaded on demand)

## Migration

### CLI

```bash
# Migrate from local to S3
packt migrate /var/backups s3://my-bucket/backups

# Migrate between S3 buckets
packt migrate s3://old-bucket/data s3://new-bucket/data
```

### Library

```rust
use packt_lib::store::{Store, StoreConfig};

let src = Store::open(StoreConfig::Local { path: "./old-store".into() })?;
let dst = Store::open(StoreConfig::S3 {
    bucket: "my-bucket".into(),
    region: Some("us-east-1".into()),
    ..Default::default()
})?;

let files = src.list_files()?;
for f in &files {
    src.restore("/tmp/migrate".as_ref(), Some(&f.name))?;
    dst.backup(
        format!("/tmp/migrate/{}", f.name).as_ref(),
        &packt_lib::store::BackupOpts { force: true, ..Default::default() },
    )?;
}
```

Current migration restores files to a temp directory and re-backups them.
Chunk-level copy (faster, no re-chunking) is planned for a future release.

## Feature Flags

| Feature | Dependencies Added | Enables |
|---------|-------------------|---------|
| (none) | — | LocalStore, Store API, all dedup/delta features |
| `cloud` | opendal, lru, tokio | S3/GCS backends, CloudStore |

```toml
# Minimal local-only
packt-lib = "0.6"

# Full cloud support
packt-lib = { version = "0.6", features = ["cloud"] }
```

## Docker/CI Cache Pattern

Use packt as a caching layer for Docker build cache or CI artifacts:

```rust
use packt_lib::store::{Store, StoreConfig, BackupOpts};
use packt_lib::types::Hash;

fn cache_restore(store: &Store, key: &str, output: &str) -> Result<bool> {
    if store.has_file(key)? {
        store.restore(output.as_ref(), Some(key))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn cache_save(store: &Store, key: &str, path: &str) -> Result<()> {
    store.backup(path.as_ref(), &BackupOpts {
        force: true,
        ..Default::default()
    })?;
    Ok(())
}
```

For immutable artifact caches (e.g., Docker layers), the content-addressed
nature means identical layers automatically deduplicate even with different keys.

## Production Considerations

### OpenDAL Configuration

When using cloud stores, configure retries and timeouts on the OpenDAL operator:

```rust
use opendal::{Operator, layers::RetryLayer};
use opendal::services::S3;

let builder = S3::default()
    .bucket("my-bucket")
    .region("us-east-1");
let op = Operator::new(builder)?
    .layer(RetryLayer::new().with_max_times(3))
    .finish();

let bop = opendal::blocking::Operator::new(op)?;
// Pass bop directly to CloudStore::open()
```

### Concurrent Access

Multiple processes accessing the same store concurrently:
- **Local**: Not safe (use OS-level locks or dedicated service)
- **S3/GCS**: Safe for reads; writes use sequential pack IDs (concurrent
  writers produce separate packs; no locking currently — Phase 4e)

### What's Not Yet Supported

- **Garbage Collection**: Chunks are never deleted. Phase 4d.
- **Encryption**: Per-chunk AEAD planned for Phase 4d.
- **Concurrent Writers**: Phase 4e.
- **GC / Pruning**: Prune old snapshots. Phase 4d.
- **Docker Registry Proxy**: Phase 4c.
- **C FFI**: Phase 4e.
