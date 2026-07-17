# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-07-17

### Added (Phase 4b — Cloud Storage + High-Level Store API)

#### Cloud S3/GCS backend (feature-gated behind `cloud`)
* **CloudStore** — Full `ContentStore` implementation for S3 and GCS via OpenDAL.
  Feature-gated behind `cloud` to keep dependencies minimal for local-only users.
* **OpenDAL integration** — Uses Apache OpenDAL v0.57 with `services-s3`,
  `services-gcs`, and `blocking` features. Unified API for both backends.
* **Pack strategy** — Same 16MB pack files as LocalStore. Packs serialized via
  `pack::write_pack` and uploaded as single S3/GCS objects.
* **Global index** — `packs/_meta.index` maintained alongside packs for fast
  index rebuild on open. Falls back to pack footer scan if missing.
* **Optional local LRU cache** — `cache_dir` parameter caches downloaded packs
  locally. LRU eviction (default 128 packs ~ 2GB). Avoids re-downloading
  frequently accessed chunks.
* **`PacktError::Cloud`** variant — Wraps OpenDAL errors with context.

#### High-level Store API (no feature gate needed for Local)
* **`Store` enum** — Facade over `LocalStore` / `CloudStore` backends.
* **`StoreConfig`** — `#[non_exhaustive]` enum: `Local { path }`,
  `S3 { bucket, region, endpoint, ... }`, `GCS { bucket, prefix, ... }`.
* **`Store::open(config)`** — Opens any backend from config.
* **`Store::backup(source, opts)`** — Full pipeline + manifest write.
  Supports incremental backup via mtime+size check.
* **`Store::restore(dest, file?)`** — Restore one or all files with mtime preservation.
* **`Store::list_files()`** — List backed-up files with size, mtime, chunk count.
* **`Store::info()`** — File count, total bytes, total chunks.
* **`Store::verify(file?)`** — Verify all chunks via BLAKE3 checksum.
* **`Store::delete_file(name)`** — Remove manifest (no GC yet — Phase 4d).
* **`Store::has_file(name)`** — Check if file exists.
* **`Store::iter_chunks()`** — Stub for future GC (Phase 4d).
* **`Store::config_from_uri(uri)`** — Parse `s3://`, `gcs://`, `/path` URIs.
* **Manifest storage** — JSON manifests stored in `manifests/` dir (local) or
  `manifests/` prefix (cloud).

#### CLI improvements
* **URI-style store paths** — All store arguments accept `/local/path`,
  `s3://bucket/key`, or `gcs://bucket/prefix`.
* **`packt migrate <src> <dst>`** — Migrate data between any backends.
  Uses restore-to-temp + backup-from-temp (chunk-level copy planned).
* **CLI simplification** — All commands rewritten to use `Store` API.
  Net deletion of 391 lines.

#### Internal fixes
* **`get_inner()` fall-through** — `LocalStore` and `CloudStore` now fall
  through to linear scan when index has stale placeholder entries (fixes
  intra-process restore after backup).
* **Phase 4a fix verified** — The O(1) index bug (placeholder `PackLocation`
  with offset=0 after flush) is now handled gracefully.

### Dependencies
* `opendal` v0.57 (optional, `cloud` feature) — Unified S3/GCS access
* `lru` v0.12 (optional, `cloud` feature) — LRU cache eviction
* `tokio` v1 (optional, `cloud` feature, `rt` feature) — Runtime for OpenDAL
* `filetime` v0.2 (new core dep) — Mtime restoration
* `serde_json` v1 (new core dep) — Manifest serialization
* `tempfile` v3 (CLI dep) — Temp dir for migrate command

## [0.5.2] - 2026-07-17

### Added

* **`packt list <store>`** -- List all backed up files with size and chunk count.
  Enables API-based extraction without guessing file names.
* **`packt restore <store> <dest> <file>`** -- Restore a single file by name without
  decompressing the entire store. Omit the file name to restore all files.
* **Incremental backup** -- `packt backup` now skips files that are unchanged since
  the last backup (compares mtime + file size against stored manifest). Use `--force`
  to re-backup regardless.
* **Per-crate README files** -- Separate README.md for `packt-lib` (library users)
  and `packt-cli` (CLI users), linked into docs.rs via `include_str!`.

### Fixed

* **Chunk hashes not saved in manifest** -- `stats.chunk_hashes` assignment was lost
  during pipeline revert. Manifests now correctly record all chunk hashes for restoration.

## [0.5.1] - 2026-07-17

### Added

* Per-crate README.md for packt-lib and packt-cli.
* `readme` and `homepage` fields in Cargo.toml.
* `#![doc = include_str!("../README.md")]` in lib.rs for docs.rs display.

## [0.5.0] - 2026-07-17

### Added (Phase 4a — Core Fixes + Production Hardening)

* **DedupIndex-backed `get()`** — `LocalStore::get()` now uses DedupIndex for O(1)
  chunk lookup instead of O(n) linear scan of all packs. Restore speed no longer
  degrades as store grows. Index set via `LocalStore::set_index()`.
* **Sharded PalantirIndex** — `ShardedPalantirIndex` with 4 hash-prefix shards
  enables concurrent similarity queries without Mutex contention. Each shard has
  its own LRU eviction and memory budget.
* **PalantirIndex query limiting** — Candidates capped at 5 per super-feature
  match (max 65 candidates per query). Query time stays constant regardless of
  index size.
* **`export_entries()` on PalantirIndex** — Enables rebuilding the sharded index
  from a non-sharded source.
* **Property tests** — Delta roundtrip (1000+ random cases), CDC determinism
  (1000+ random cases), pack roundtrip (random entries).
* **Fuzz targets** — Pack reader no-panic test on arbitrary data, delta codec
  no-panic test on arbitrary data.

### Changed

* **Store `put()` O(n) → O(1)** — Replaced linear scan over all pack entries
  with DedupIndex lookup. Same fix as `get()` — both were O(n) per operation,
  making the writer thread O(n^2) overall.
* **Store `contains()` O(n) → O(1)** — Same fix. Index checked first, pack
  scan kept as fallback for safety.
* **Incompressible data fast-path** — Quick entropy check (sample 1KB, count
  unique byte values) skips zstd for random/incompressible data. Prevents
  wasted CPU on zstd hash-table searches.
* **Pipe-through verification** — Remove redundant read-back in pack flush.
  `flush_write()` now verifies from the in-memory buffer instead of re-reading
  the just-written file from disk.
* **zstd level 3 (was 7)** — Level 3 is 2-3x faster with ~5% ratio loss.
  Level 7 was over-optimized for the dedup use case where zstd only touches
  unique chunks (typically 15-25% of data after dedup).
* **zstdmt feature** — Multi-threaded zstd for super-block compression (4
  workers). Speeds up large-pack flush.
* **SimilarityStage** — Now uses `ShardedPalantirIndex` internally. No external
  Mutex needed — sharding provides internal concurrency.
* **`#[allow(dead_code)]` removed** from `config` field in `BackupPipeline`.
* `cargo test` count: 73 → 78 tests (unit + integration + property + fuzz).

### Removed

* **Parallel pipeline (Rayon par_iter)** — Measured no wall-time benefit.
  BLAKE3 of 32KB takes ~2us per chunk. Even 60K chunks = 0.12s CPU total,
  dwarfed by FastCDC chunking (1.4 GB/s, sequential) and zstd compression.
  Pipeline restored to simple sequential loop.
* **Progress callback** — Zero consumers. Speculative API. Add when a library
  consumer needs it.
* **FPR byte-frequency histogram** — Dead code. `check_similarity()` is
  defined but never called from the pipeline. Similarity stage uses
  `PalantirIndex.query()` directly.

### Performance (v0.5.0, zstd level 3)

| Workload | v0.4.0 (lvl7) | v0.5.0 (lvl3) | Change |
|---|---|---|---|
| Docker layers (5x, 363MB) | ~70 MB/s (5.2s) | 118 MB/s (3.07s) | +69% |
| 500MB zeros | ~833 MB/s | 1.66 GB/s | +100% |
| 500MB random | ~227 MB/s | 249 MB/s | +10% |
| 2GB random | ~232 MB/s | 262 MB/s | +13% |

Storage efficiency (Docker layers):
  v0.4.0 (lvl7): 89MB stored, 4.1x ratio
  v0.5.0 (lvl3): 90MB stored, 4.0x ratio
  Difference: 1.1% (negligible for dedup workloads)

All improvements come from zstd level tuning, entropy-pass, and removing
redundant I/O. The pipeline remains sequential — FastCDC chunking at 1.4 GB/s
is the primary bottleneck for single-file processing.

### Added (Phase 3 — Delta Compression)

* **Pack format v3 (PACKv3)** — Super-block compression: Full/FullRaw chunk data
  concatenated and compressed as a single zstd frame, enabling cross-chunk
  pattern matching. 5--15% additional compression ratio improvement.
* **EntryType::FullRaw** — Chunks where zstd compression expands the data
  are stored raw with a type marker. Zero CPU wasted on incompressible data.
* **Cross-session similarity persistence** — Palantir signatures stored in
  pack index entries, rebuilt on store open. Near-dup detection works across
  CLI invocations.
* **BLAKE3 parallel batch hashing** — `rayon` feature enabled on BLAKE3,
  `hash_batch()` method for multi-core hashing throughput.
* **Content-aware compression** — zstd level 7 upgrade (from level 3) with
  automatic fallback to raw storage when compression ratio drops below 95%
  of original size.

### Added (Phase 2 — Near-Duplicate Detection)

* **Palantir hierarchical super-feature similarity engine**
  * Replaces expensive MinHash+LSH (8s/200 MB overhead to ~0 ms)
  * 12 fixed sub-chunk feature extraction with xxh3 sampling
  * 3-tier matching: Tier 1 (>=95%), Tier 2 (>=85%), Tier 3 (>=70%)
  * True LRU eviction with touch-on-access
  * Best-match candidate ranking by super-feature count
  * False-positive filter via head+tail xxh3 comparison
  * 104 bytes per entry (vs 960 for MinHash)
* **Delta compression** — zstd dictionary mode for similar chunks with
  automatic fallback when delta >90% of standalone compression size.
* **Pipeline integration** — SimilarityStage between DedupStage and WriterStage
* **CLI** — `--similarity-threshold` flag (default 0.7, 0 = disable)
* **Backup stats** — Near-duplicate chunk count, delta compressed chunks,
  delta savings bytes, delta fallback count
* **Benchmark scripts** — `phase3_comprehensive_benchmark.sh`,
  `phase2_similarity_test.sh`
* **Real-world benchmarks** — 4.4x on cross-image Docker, 4.0x on daily
  backups, 8.1x on VM snapshots. 36% better storage than restic on cross-
  image workloads.

### Changed

* **Compression level** — zstd level raised from 3 to 7 (25% better
  compression on compressible data with 80% write speed).
* **Pack format** — PACKv3 with super-block compression. Backward compatible:
  reads PACKv1 and PACKv2 formats.
* **Dependencies** — Added `gdelta` (evaluated, decode bug found — disabled),
  added `rayon` feature to `blake3`.
* **Similarity index** — Now persisted across CLI invocations via pack index
  signatures.

### Removed

* `similarity/shingle.rs`, `minhash.rs`, `lsh.rs` — entire MinHash+LSH module
  replaced by Palantir (Phase 2).

## [0.4.0] - 2026-07-17

### Added

* **Palantir hierarchical super-feature similarity engine** — Phase 2
  * Replaces expensive MinHash+LSH (8s/200 MB overhead to ~0 ms)
  * 12 fixed sub-chunk feature extraction with xxh3 sampling
  * 3-tier matching: Tier 1 (>=95%), Tier 2 (>=85%), Tier 3 (>=70%)
  * True LRU eviction with touch-on-access
  * Best-match candidate ranking by super-feature count
  * False-positive filter via head+tail xxh3 comparison
  * 104 bytes per entry (vs 960 for MinHash)
* **Pipeline integration**: SimilarityStage between DedupStage and WriterStage
* **CLI**: `--similarity-threshold` flag (default 0.7). Set to 0 to disable.
* **Backup stats**: Near-duplicate chunk count, similarity tier info
* **Tests**: 23 new unit tests + 3 integration tests (70 total)
* **scripts**: `phase2_similarity_test.sh` (Docker + synthetic),
  `compare_all.sh` (packt vs restic)
* `xxhash-rust` dependency for fast non-crypto hashing

### Changed

* **DECISIONS.md**: Updated D004 (Palantir), added D016-D020, Phase 3 pack
  format spec
* **Dependency**: `xxhash-rust` 0.8 added (removes need for MinHash crates)

### Removed

* `similarity/shingle.rs`, `minhash.rs`, `lsh.rs` — entire MinHash+LSH module
  replaced

## [0.3.0] - 2026-07-16

### Changed

* **Project rename**: `dedup` to `packt` (binary, crates, error types, all
  references)
* **CI**: Fixed Windows build (Unix permission gating), bench target, audit
  caching
* **Git workflow**: Added PR-only rule — never push to main directly

### Added

* Streaming pipeline via `fastcdc::StreamCDC` — ~8 MB peak memory, no full
  file load
* Cross-platform helpers for Unix permissions (`#[cfg(unix)]`)

### Removed

* `SourceReader` (replaced by streaming)
* `ChunkerStage` (replaced by StreamCDC)
* `reader.rs`, `chunker_stage.rs` modules

## [0.2.0] - 2026-07-16

### Changed

* **Crate rename**: `compressor-lib` to `dedup-lib`, `compressor-cli` to
  `dedup-cli`, binary `compressor` to `dedup`
* **Error type rename**: `CompressionError` to `DedupError`
* **CI/CD overhaul**: Production-grade pipeline with 8 quality gates

### Added (CI/CD)

* **Release workflow**: Automated crates.io publish + GitHub release on
  `v*` tag
* **Security workflow**: `cargo-deny` for advisories + licenses + bans
* **Code coverage**: `cargo-llvm-cov` with Codecov upload
* **Documentation build check**: `cargo doc` with `-D warnings`
* **MSRV verification**: CI job building at `rust-version = "1.85"`
* **Beta toolchain**: Added to test matrix for early regression detection
* **Concurrency control**: `cancel-in-progress: true` to save runner minutes
* **Reproducible builds**: `--locked` on all cargo commands
* **Contributor files**: Bug/feature issue templates, PR template,
  CONTRIBUTING.md, SECURITY.md
* **Config files**: `deny.toml`, `.cargo/config.toml`, `clippy.toml`,
  `rustfmt.toml`

### Changed (CI/CD)

* **ci.yml rewritten**: From 75 to 108 lines. Combined fmt+clippy into
  single lint job. Removed redundant `build-all` job.
* **Audit moved**: From inline `cargo install cargo-audit` to
  `cargo-deny-action` in security.yml.
* **`rust-toolchain.toml`**: Added `llvm-tools-preview` component for
  coverage tooling.

### Fixed (library)

* **Removed `unwrap()` in library code**: HashIndex bloom filter lock and
  hash conversion now use `expect()` with descriptive messages.
* **bincode to postcard**: Replaced defunct bincode v2 with maintained
  postcard for pack serialization
* **dependencies**: Removed `memmap2` (unused), removed `zstd` experimental
  feature (premature for Phase 1)

### Fixed

* **Bloom filter now operational**: Wrapped in `Mutex`, wired into
  `HashIndex::insert()` and `lookup()`.
* **Index persistence**: Added `populate_index()` to `LocalStore`, called on
  startup so the dedup index is pre-populated from existing packs
* **Coverage validation**: `debug_assert!` to `assert!` in chunk boundary
  checks (critical for release build safety)
* **`store.get()` no longer holds lock during disk I/O**: Restructured to
  find location under lock, then release before reading
* **Writer thread now updates index**: New chunks inserted into the HashIndex
  immediately after storage, enabling intra-file dedup
* **Worker thread `send()` errors no longer silently discarded**

### Added (Phase 1 hardening)

* **File metadata in backup manifests**: Stores path, size, modification
  time, permissions alongside chunk hashes
* **Metadata restoration**: Restore command now preserves file mtime and
  permissions
* **Backward compat**: Old manifests (bare hash lists) are still readable
* **Criterion benchmarks**: `chunking_throughput`, `hashing_throughput`,
  `pack_roundtrip`
* `pub use error::Result as PacktResult` for library users

### Removed

* `BufferPool` (dead code)
* `StoredChunk` type (dead code)
* `WriterStage` module (unused)
* `memmap2` dependency (unused)
* `util::buffer` module

## [0.1.0] - 2026-07-16

### Added

* Initial project structure with workspace layout
* FastCDC v2020 content-defined chunking
* BLAKE3 content hashing with known-vector tests
* Content-addressed pack format with integrity verification
* Local filesystem store with atomic write semantics
* Concurrent dedup index with DashMap backend
* Pipeline architecture for backup/restore workflows
* CLI with backup, restore, info, verify, benchmark subcommands
* Property-based tests using proptest
* CI pipeline: fmt, clippy, test (3 OS), bench, audit
