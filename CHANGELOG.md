# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2026-07-17

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
