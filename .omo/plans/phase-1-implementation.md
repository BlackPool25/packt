# Phase 1 Implementation Plan: CDC + Exact Dedup

> **Status**: Draft (awaiting approval)
> **Created**: 2026-07-16
> **Source Decisions**: DECISIONS.md (D001-D015)
> **Research Reference**: PROJECT_RESEARCH_COMPLETE.md Section 2 (CDC), Section 5 (Rust Ecosystem), Section 7 (Architecture)
> **Agent Rules**: RULES.md

---

## 1. Overview

**Goal**: Working backup/restore CLI that chunks files with FastCDC v2020, deduplicates exact matches via BLAKE3 content-addressing, stores in a header-at-end pack format, and restores bit-exact output.

**Provides value alone**: 25%+ storage reduction on cross-version backups (comparable to restic/kopia for exact dedup use cases).

**Architecture**: Single Rust binary (`compressor`) with library crate (`compressor-lib`).

---

## 2. Project Structure

```
compressor/
├── Cargo.toml                  # Workspace root
├── rust-toolchain.toml         # Pin stable Rust
├── .gitignore
├── CHANGELOG.md
├── README.md
├── LICENSE                     # MIT OR Apache 2.0
├── .github/
│   └── workflows/
│       └── ci.yml              # CI: fmt, clippy, test, bench, audit
│
├── compressor-lib/             # Library crate (the product)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              # Re-exports, #![forbid(unsafe_code)]
│       ├── error.rs            # CompressionError enum
│       ├── types.rs            # Chunk, Hash, PackLocation, ChunkInfo
│       ├── chunking/
│       │   ├── mod.rs          # Chunker trait
│       │   └── fastcdc.rs      # FastCDC v2020 impl
│       ├── hash/
│       │   ├── mod.rs          # Hasher trait
│       │   └── blake3.rs       # BLAKE3 wrapper
│       ├── store/
│       │   ├── mod.rs          # ContentStore trait
│       │   ├── pack.rs         # PackFormat read/write
│       │   └── local.rs        # Local filesystem backend
│       ├── index/
│       │   ├── mod.rs          # Index trait
│       │   └── hashindex.rs    # DashMap-based concurrent index
│       ├── pipeline/
│       │   ├── mod.rs          # Pipeline orchestrator
│       │   ├── reader.rs       # File reading stage
│       │   ├── chunker.rs      # Chunking stage wrapper
│       │   ├── hasher.rs       # Hashing stage wrapper
│       │   ├── dedup.rs        # Dedup check stage wrapper (Phase 1: exact only)
│       │   └── writer.rs       # Pack writing stage
│       └── util/
│           ├── mod.rs
│           └── buffer.rs       # Reusable buffer pool
│
├── compressor-cli/             # CLI binary crate (the demo/dogfood)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # Entry point, clap setup
│       ├── backup.rs           # backup subcommand
│       ├── restore.rs          # restore subcommand
│       ├── info.rs             # info subcommand
│       └── verify.rs           # verify subcommand
│
├── tests/                      # Integration tests (crate-level)
│   ├── common/
│   │   └── mod.rs              # Test helpers, test corpus paths
│   ├── integration_test.rs     # End-to-end backup/restore test
│   └── real_world.rs           # Real-world Docker layer test
│
└── benches/
    └── pipeline.rs             # Criterion benchmarks for each stage
```

---

## 3. Data Types & Traits (Shared Contract)

Defined in `compressor-lib/src/types.rs` and respective `mod.rs` files. These are the **interface contracts** that all subagents implement against.

### 3.1 Core Types (types.rs)

```rust
/// A content-defined chunk of data
#[derive(Debug, Clone)]
pub struct Chunk {
    pub offset: u64,       // Position in source file
    pub length: u32,       // Size of chunk data
    pub data: Vec<u8>,     // The chunk bytes (populated during read, cleared after hash/store)
}

/// BLAKE3 hash wrapper (32 bytes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn from_blake3(hash: blake3::Hash) -> Self { ... }
    pub fn to_hex(&self) -> String { ... }
}

/// Location of a chunk in a pack file
#[derive(Debug, Clone, Copy)]
pub struct PackLocation {
    pub pack_id: u32,       // Which pack file
    pub offset: u64,        // Byte offset in pack
    pub length: u32,        // Compressed length
    pub orig_length: u32,   // Original uncompressed length
}

/// Metadata about a stored chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub hash: Hash,
    pub location: PackLocation,
    pub is_compressed: bool,
}
```

### 3.2 Traits

```rust
/// Content-Defined Chunking
pub trait Chunker: Send + Sync {
    fn chunk(&self, data: &[u8]) -> Vec<Chunk>;
    fn chunk_config(&self) -> ChunkConfig;
}

/// Content hashing
pub trait ContentHasher: Send + Sync {
    fn hash(&self, data: &[u8]) -> Hash;
    fn hash_chunk(&self, chunk: &Chunk) -> Hash;
}

/// Content-addressed storage backend
#[async_trait]  // or sync trait
pub trait ContentStore: Send + Sync {
    fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation>;
    fn get(&self, hash: &Hash) -> Result<Vec<u8>>;
    fn contains(&self, hash: &Hash) -> Result<bool>;
    fn flush(&self) -> Result<()>;
}

/// Dedup index
pub trait DedupIndex: Send + Sync {
    fn insert(&self, hash: Hash, location: PackLocation);
    fn lookup(&self, hash: &Hash) -> Option<PackLocation>;
    fn contains(&self, hash: &Hash) -> bool;
    fn len(&self) -> usize;
}
```

---

## 4. Implementation Steps (Subagent Tasks)

### Phase 0: Project Scaffolding (Main Thread)

**Files**: Cargo.toml (both), rust-toolchain.toml, .gitignore, LICENSE, README.md, CHANGELOG.md, .github/workflows/ci.yml, shared types, error module, traits

**Done by**: Main orchestrator (me).

**Dependencies**: None (this IS the foundation).

**Deliverables**:
- Working `cargo build`
- Shared type definitions (Chunk, Hash, PackLocation, ChunkInfo)
- All traits defined (Chunker, ContentHasher, ContentStore, DedupIndex)
- Error types defined
- CI workflow green
- Cargo.toml with ALL dependencies declared (so subagents just import)

---

### Step A: Chunking Module (Independent Subagent)

**Files**: `compressor-lib/src/chunking/mod.rs`, `compressor-lib/src/chunking/fastcdc.rs`

**Interface to implement**: `Chunker` trait

**Depends on**: Types from Phase 0 (Chunk, ChunkConfig, CompressionError)

**Does NOT depend on**: Hash, Store, Index, CLI — completely independent

**Tests**:
- Determinism: same input → same chunk boundaries (proptest, 1000 random inputs)
- Boundary shift: insert byte at start → subsequent boundaries re-synchronize within N chunks
- Empty input: produces one chunk (or zero, per policy)
- All-zeros: no infinite loop, bounded chunk sizes
- Chunk coverage: sum of chunk lengths == input length, no gaps, no overlaps
- Configuration bounds: test min=avg, avg=max, extreme values
- **Real-world**: chunk a 100MB binary file, verify boundaries are deterministic across runs

**Benchmarks**:
- Throughput (MB/s) at 4KB, 16KB, 32KB, 64KB, 128KB avg sizes
- Throughput comparison: v2016 vs v2020 vs ronomon variants
- Memory usage during chunking

**Criterion benchmarks**: `chunking_throughput`, `chunking_determinism`, `chunking_distribution`

**Est. time**: 2 days for careful implementation + tests + benches

---

### Step B: Hashing Module (Independent Subagent)

**Files**: `compressor-lib/src/hash/mod.rs`, `compressor-lib/src/hash/blake3.rs`

**Interface to implement**: `ContentHasher` trait

**Depends on**: Types from Phase 0 (Hash, Chunk, CompressionError)

**Does NOT depend on**: Chunking, Store, Index, CLI

**Tests**:
- Known test vectors: blake3 hash of empty string, "abc", known binary patterns
- Determinism: same input → same hash (proptest, 1000 random inputs)
- Collision resistance: different inputs → different hashes (property, 10000 random pairs)
- Streaming: hash large input via incremental update matches one-shot hash
- Keyed hashing mode works (for future domain separation)

**Benchmarks**:
- Throughput (MB/s) for various input sizes (1KB to 64MB)
- Comparison: BLAKE3 vs SHA-256 throughput

**Criterion benchmarks**: `hashing_throughput`, `hashing_streaming`

**Est. time**: 0.5 day

---

### Step C: Pack Format (Independent Subagent)

**Files**: `compressor-lib/src/store/pack.rs`

**Interface to implement**: Pack read/write functions (internal, not a trait — used by ContentStore)

**Depends on**: Types from Phase 0 (Hash, PackLocation, ChunkInfo, CompressionError)

**Does NOT depend on**: Chunking, Hash implementation, CLI

**Implements the format**:
```
[Entry 1: zstd-compressed chunk data]
[Entry 2: zstd-compressed chunk data]
...
[Entry N: zstd-compressed chunk data]
[Index section: bincode-serialized Vec<ChunkInfo>]
[Footer: index_offset(u64), index_len(u32), checksum([u8; 32]), magic(b"PACKv1")]
```

**Key design decisions**:
- Index at end allows streaming writes without knowing final size
- Footer at very end for quick validation
- Each chunk compressed independently (zstd level 3 default)
- Index is bincode-serialized (fast binary, not human-readable)
- Footer checksum covers everything before it

**Tests**:
- Round-trip: serialize pack with N mock chunks → deserialize → verify all match
- Empty pack: 0 chunks → valid empty pack → deserialize to empty list
- Single chunk: 1 chunk → round-trip
- Large number of chunks (10000+) → round-trip, verify all hashes
- Footer checksum integrity: corrupt one byte → verification fails
- Partial write detection: truncated data → deserialization error
- Boundary conditions: max u32 length, max u64 offset

**Est. time**: 2 days

---

### Step D: Content Store + Index (Independent Subagent)

**Files**: 
- `compressor-lib/src/store/mod.rs` (ContentStore trait)
- `compressor-lib/src/store/local.rs` (LocalStore implementation)
- `compressor-lib/src/index/mod.rs` (DedupIndex trait)
- `compressor-lib/src/index/hashindex.rs` (HashIndex implementation)

**Interfaces to implement**: `ContentStore` trait, `DedupIndex` trait

**Depends on**: Types from Phase 0 + Pack format types (PackLocation, ChunkInfo)

**Does NOT depend on**: Chunking, Hashing implementation, CLI

**LocalStore details**:
- Directory layout: `{store_root}/packs/{pack_id}.pack` + `{store_root}/index.bin`
- Atomic writes: write to `.tmp` → fsync → rename to final
- Pack creation: accumulate chunks in memory buffer (up to `PACK_TARGET_SIZE` = 16MB), flush to disk
- Read: load pack index on open, seek to chunk offset + decompress

**HashIndex details**:
- Backed by `dashmap::DashMap<Hash, PackLocation>` for concurrent access
- Bloom filter for fast negative checks (reduce DashMap lookups by 99%)
- `bloom` crate or custom fixed-size bloom filter
- Bloom filter: 10 bits per entry, optimal k hashes via BLAKE3 (truncate output to k keys)
- Thread-safe (DashMap handles sharded locking)

**Tests**:
- Store + retrieve round-trip: put chunk → get chunk → verify data matches
- Store dedup: put same hash twice → only stored once (no duplicate entry)
- Missing chunk: get nonexistent hash → None/NotFound error
- Concurrent store/retrieve: 16 threads simultaneously, no data loss
- Index insert + lookup: 1M entries, measure lookup latency
- Bloom filter: verify no false negatives (all inserted keys pass filter)
- Pack flush: partial flush → resume → verify pack is valid
- Crash recovery: kill during pack write → verify no corrupt packs on restart

**Benchmarks**:
- Store throughput (chunks/s) at various chunk sizes
- Index lookup throughput (lookups/s) at various index sizes
- Memory usage per million entries

**Criterion benchmarks**: `store_throughput`, `index_lookup_latency`, `bloom_filter_accuracy`

**Est. time**: 3 days

---

### Step E: CLI Binary (Independent Subagent)

**Files**: All files in `compressor-cli/src/`

**Interface to consume**: Library public API from `compressor-lib`

**Depends on**: The exported public API of compressor-lib (Pipeline, types)

**Does NOT depend on**: Internal module structure — only depends on the public API

**Subcommands**:
```
compressor backup <source> <store-dir>
    - Reads source file/directory
    - Chunks, hashes, deduplicates, stores
    - Progress bar via `indicatif`
    - Stats at end: unique chunks, duplicates skipped, compression ratio, throughput

compressor restore <store-dir> <output-dir>
    - Loads backup metadata from store
    - Reconstructs all files
    - Verifies each chunk hash on read
    - Output: original files bit-exact

compressor info <store-dir>
    - Total chunks (unique + total stored)
    - Total data in / out (compression ratio)
    - Pack file count and sizes
    - Index memory usage

compressor verify <store-dir>
    - Reads every chunk from packs
    - Recomputes BLAKE3 hash and compares with stored hash
    - Reports any corruption
    - Exit code 0 = all good, 1 = corruption found

compressor benchmark <corpus-dir>
    - Runs internal benchmarks on provided data
    - Reports throughput per stage
    - CSV output for analysis
```

**Tests**:
- Snapshot testing: run CLI with known input, compare output with golden files (`insta` crate)
- Help output: `--help` for each subcommand, verify expected flags present
- Error handling: invalid paths, missing arguments → informative error messages
- **Real-world**: actual backup + restore of a real directory tree

**Est. time**: 2 days

---

### Step F: Pipeline Integration (Main Thread)

**Files**: `compressor-lib/src/pipeline/` (all files)

**Interface to implement**: Pipeline orchestrator

**Depends on**: ALL modules (chunking, hashing, store, index) — this is the integration layer

**Architecture**:
```
Reader (mmap file in chunks)
  → crossbeam channel (bounded, capacity=32)
    → Chunker stage (FastCDC, produces Vec<Chunk>)
      → crossbeam channel (bounded)
        → Hasher stage (BLAKE3 per chunk)
          → crossbeam channel (bounded)
            → Dedup stage (hash index lookup + bloom filter)
              → crossbeam channel (bounded)
                → Writer stage (zstd compress + pack assemble + store)
```

**Backpressure**: Bounded channels naturally block upstream when downstream is full.

**Parallelism**: 
- Reader: 1 thread (IO-bound)
- Chunker: rayon pool (CPU-bound)
- Hasher: rayon pool (CPU-bound)
- Dedup: DashMap concurrent access (lock contention bound)
- Writer: 1 thread (sequential IO)

**Error handling**: Any stage error → shutdown pipeline → return error to caller. No silent failures.

**Tests**:
- End-to-end: backup small file → restore → verify identical
- End-to-end: backup directory with multiple files → restore → verify all
- Empty file backup/restore
- Large file (1GB+) backup/restore
- Interrupted backup: kill mid-way → verify no corrupt state

**Est. time**: 2 days

---

### Step G: Integration & Real-World Tests

**Files**: `tests/integration_test.rs`, `tests/real_world.rs`, `tests/common/mod.rs`

**Integration tests** (run automatically in CI):
- `test_backup_restore_file`: backup single file, restore, compare SHA-256
- `test_backup_restore_dir`: backup dir with mixed sizes (1B to 100MB), restore, compare
- `test_backup_restore_empty`: empty file backup/restore
- `test_backup_restore_large`: 1GB file via mmap
- `test_verify_corrupt`: corrupt a pack byte, verify flag
- `test_dedup_repeated`: same file backed up twice, verify only stored once
- `test_concurrent_backup`: two simultaneous backups to same store

**Real-world test** (manual, documented in README):
```
Test name: wt_docker_layer_dedup
Corpus: docker pull ubuntu:22.04, 23.04, 23.10 (or similar sequential versions)
          Extract layers, run compressor backup on all layers
Metrics:
  - Unique chunks found
  - Total chunks skipped (dedup ratio)
  - Compression ratio (original bytes / stored bytes)
  - Backup throughput (MB/s)
  - Restore throughput (MB/s)
  - Restore verification (all files SHA-256 match originals)

Expected: 30-50% storage reduction from exact dedup alone
          (identical packages shared across Ubuntu versions)

Comparison: Run restic backup on same corpus
  - How does dedup ratio compare?
  - How does throughput compare?
```

**Script**: `scripts/real_world_test_docker.sh` — automated script that:
1. Pulls N versions of an image
2. Extracts layer tars
3. Runs compressor backup
4. Runs compressor restore + verify
5. Reports all metrics

**Est. time**: 2 days

---

## 5. Subagent Task Independence Matrix

| Task | Depends On | Can run in parallel with | Independent test strategy |
|------|-----------|-------------------------|--------------------------|
| Phase 0: Scaffold | Nothing | Everything else | N/A (foundation) |
| Step A: Chunking | Phase 0 types | B, C, D, E | Test with raw byte arrays, no hashing needed |
| Step B: Hashing | Phase 0 types | A, C, D, E | Test with known test vectors, no chunker needed |
| Step C: Pack Format | Phase 0 types | A, B, D, E | Test with mock chunk data, no chunker/hasher needed |
| Step D: Store+Index | Phase 0 types | A, B, C, E | Test with mock hashes, no chunker/hasher needed |
| Step E: CLI | Phase 0 types | A, B, C, D | Test with mock store (struct), no real pipeline needed |
| Step F: Pipeline | A, B, C, D | E (can be parallel) | Integration requires all modules |
| Step G: Integration | F, E | Nothing | End-to-end real tests |

**Parallel execution plan**:
- Wave 1 (all parallel): Steps A, B, C, D, E — 5 subagents simultaneously
- Wave 2 (sequential after Wave 1): Step F (integrates A+B+C+D outputs)
- Wave 2 parallel: Step E can still be refined
- Wave 3 (after F): Step G (integration tests)

---

## 6. CI Configuration

`.github/workflows/ci.yml`:
```yaml
name: CI
on: [push, pull_request]
jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - run: cargo fmt --check
  clippy:
    runs-on: ubuntu-latest
    steps:
      - run: cargo clippy -- -D warnings
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - run: cargo test --all-features
  bench:
    runs-on: ubuntu-latest
    steps:
      - run: cargo bench -- --quick     # Quick mode in CI (fewer samples)
  audit:
    runs-on: ubuntu-latest
    steps:
      - run: cargo audit                # Security audit of dependencies
  msrv:
    runs-on: ubuntu-latest
    steps:
      - run: cargo build                # Verify latest stable compiles
```

---

## 7. Dependencies (Cargo.toml)

```toml
[workspace]
members = ["compressor-lib", "compressor-cli"]

[package]
name = "compressor-lib"
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
description = "Content-defined chunking with exact dedup for binary data"
repository = "https://github.com/YOUR_USER/compressor"

[dependencies]
fastcdc = "4"                    # FastCDC v2020 chunking
blake3 = "1"                     # BLAKE3 hashing
zstd = { version = "0.13", features = ["experimental"] }  # zstd compression + dict support
serde = { version = "1", features = ["derive"] }           # Serialization
bincode = "2"                    # Binary serialization (fast)
rayon = "1"                      # Work-stealing parallelism
crossbeam-channel = "0.5"        # Bounded MPMC channels
dashmap = "6"                    # Concurrent hash map
memmap2 = "0.9"                  # Memory-mapped file IO
tracing = { version = "0.1", features = ["attributes"] }  # Structured logging
thiserror = "2"                  # Error derive
cfg-if = "1"                     # Conditional compilation

[dev-dependencies]
tempfile = "3"                   # Temp directories for test isolation
criterion = { version = "0.5", features = ["html_reports"] }  # Benchmarking
proptest = "1"                   # Property-based testing
rand = "0.8"                     # Random test data generation
walkdir = "2"                    # Directory traversal for test corpus

[[bench]]
name = "pipeline"
harness = false
```

```toml
[package]
name = "compressor-cli"
version = "0.1.0"
edition = "2024"

[dependencies]
compressor-lib = { path = "../compressor-lib" }
clap = { version = "4", features = ["derive"] }  # CLI parser
indicatif = "0.17"               # Progress bars
anyhow = "1"                     # Error handling (CLI only)
tracing-subscriber = { version = "0.3", features = ["env-filter"] }  # Log output
serde = { version = "1", features = ["derive"] }
serde_json = "1"                 # JSON output for info command
```

---

## 8. Git Workflow

```
main          — Production-ready code, CI green
  └─ phase-1  — Phase 1 development branch
       ├─ step-a-chunking    — Subagent A output
       ├─ step-b-hashing     — Subagent B output
       ├─ step-c-pack        — Subagent C output
       ├─ step-d-store       — Subagent D output
       ├─ step-e-cli         — Subagent E output
       ├─ step-f-pipeline    — Integration (main thread)
       └─ step-g-tests       — Integration tests
```

Each step branch → PR into `phase-1`. Merge only after:
- CI green
- Code review (by main orchestrator)
- All tests pass
- LSP diagnostics clean

---

## 9. Verification Gates (Phase 1 Completion)

- [ ] `cargo build` — clean build, no warnings
- [ ] `cargo clippy -- -D warnings` — pedantic clippy clean
- [ ] `cargo test --all-features` — all tests pass
- [ ] `cargo bench` — benchmarks produce correct results
- [ ] `cargo audit` — no dependency vulnerabilities
- [ ] `cargo fmt --check` — formatting consistent
- [ ] **Real-world test**: Docker image layers dedup achieves 30%+ reduction
- [ ] **Real-world test**: Restore produces bit-exact output (SHA-256 match)
- [ ] **Real-world test**: Backup/restore 1GB+ file succeeds
- [ ] LEARNING.md updated with Phase 1 lessons
- [ ] All deferred items (Phase 2+ features) documented in TODO
- [ ] README.md documents usage, architecture, benchmark results

---

## 10. Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| fastcdc crate API differences from docs | Low | Medium | Check cargo doc before coding, have fallback config |
| zstd-rs experimental feature instability | Low | Low | Pin version, test experimental features separately |
| DashMap memory overhead for million+ entries | Medium | Medium | Bloom filter reduces lookups; benchmark before optimizing |
| mmap portability issues (Windows) | Medium | Medium | Fall back to read() on platforms where mmap behaves differently |
| Pipeline deadlock from bounded channels | Low | High | Use crossbeam-channel with timeout; test with varied throughput |
| Test flakiness from timing-dependent code | Medium | Low | Use deterministic mocks for timing-sensitive tests |
| Real-world test requires Docker installation | Medium | Low | Script detects Docker availability; skip if not present |

---

## 11. Approval Checklist

Before approving this plan, verify:
- [ ] Decision points match DECISIONS.md
- [ ] Subagent independence constraint is satisfied (no cross-dependencies)
- [ ] All features have test requirements specified
- [ ] Real-world testing is defined, not just unit tests
- [ ] CI/CD configuration is specified
- [ ] Risk mitigations are documented
- [ ] Timeline estimates are realistic (3-4 weeks for Phase 1)

---

*End of Plan — awaiting approval before implementation.*
