# Changelog

## [0.4.0] - 2026-07-17

### Added
- **Palantir hierarchical super-feature similarity engine** (Phase 2)
  - Replaces expensive MinHash+LSH (8s/200MB overhead → ~0ms)
  - 12 fixed sub-chunk feature extraction with xxh3 sampling
  - 3-tier matching: Tier 1 (≥95%), Tier 2 (≥85%), Tier 3 (≥70%)
  - True LRU eviction with touch-on-access
  - Best-match candidate ranking by super-feature count
  - False-positive filter via head+tail xxh3 comparison
  - 104 bytes per entry (vs 960 for MinHash)
- **Pipeline integration**: SimilarityStage between DedupStage and WriterStage
- **CLI**: `--similarity-threshold` flag (default 0.7). Set to 0 to disable.
- **Backup stats**: Near-duplicate chunk count, similarity tier info
- **Tests**: 23 new unit tests + 3 integration tests (70 total)
- **scripts**: `phase2_similarity_test.sh` (Docker + synthetic), `compare_all.sh` (packt vs restic)
- `xxhash-rust` dependency for fast non-crypto hashing

### Changed
- **DECISIONS.md**: Updated D004 (Palantir), added D016-D020, Phase 3 pack format spec
- **Dependency**: `xxhash-rust` 0.8 added (removes need for MinHash crates)

### Removed
- `similarity/shingle.rs`, `minhash.rs`, `lsh.rs` — entire MinHash+LSH module replaced

## [0.3.0] - 2026-07-16

### Changed
- **Project rename**: `dedup` → `packt` (binary, crates, error types, all references)
- **CI**: Fixed Windows build (Unix permission gating), bench target, audit caching
- **Git workflow**: Added PR-only rule — never push to main directly

### Added
- Streaming pipeline via `fastcdc::StreamCDC` — ~8 MB peak memory, no full file load
- Cross-platform helpers for Unix permissions (`#[cfg(unix)]`)

### Removed
- `SourceReader` (replaced by streaming)
- `ChunkerStage` (replaced by StreamCDC)
- `reader.rs`, `chunker_stage.rs` modules

## [0.2.0] - 2026-07-16

### Changed
- **Crate rename**: `compressor-lib` → `dedup-lib`, `compressor-cli` → `dedup-cli`, binary `compressor` → `dedup`
- **Error type rename**: `CompressionError` → `DedupError`
- **CI/CD overhaul**: Production-grade pipeline with 8 quality gates (was 6). See `cicd-pipeline-overhaul.md` plan.

### Added (CI/CD)
- **Release workflow**: Automated crates.io publish + GitHub release on `v*` tag
- **Security workflow**: `cargo-deny` for advisories + licenses + bans on PR/push + weekly schedule
- **Code coverage**: `cargo-llvm-cov` with Codecov upload
- **Documentation build check**: `cargo doc` with `-D warnings`
- **MSRV verification**: CI job building at `rust-version = "1.85"` (Edition 2024 minimum)
- **Beta toolchain**: Added to test matrix for early regression detection
- **Concurrency control**: `cancel-in-progress: true` to save runner minutes
- **Reproducible builds**: `--locked` on all cargo commands
- **`--all-features`**: All build/test/lint commands now cover feature-gated code
- **Per-cell cache keys**: Separate cache buckets per OS/toolchain to prevent collisions
- **Contributor files**: Bug/feature issue templates, PR template, CONTRIBUTING.md, SECURITY.md
- **`ci-coverage.sh`**: Local coverage script matching CI
- **Config files**: `deny.toml`, `.cargo/config.toml`, `clippy.toml`, `rustfmt.toml`

### Changed (CI/CD)
- **ci.yml rewritten**: From 75→108 lines. Combined fmt+clippy into single lint job. Removed redundant `build-all` job. Added `permissions: contents: read`, `RUSTFLAGS: -D warnings`, `timeout-minutes` per job.
- **Audit moved**: From inline `cargo install cargo-audit` in ci.yml to `cargo-deny-action` in security.yml (no compile overhead, includes license/bans/sources).
- **`rust-toolchain.toml`**: Added `llvm-tools-preview` component for coverage tooling.

### Fixed (library)
- **Removed `unwrap()` in library code**: `HashIndex` bloom filter lock and hash conversion now use `expect()` with descriptive messages.
- **bincode → postcard**: Replaced defunct bincode v2 with maintained postcard for pack serialization
- **dependencies**: Removed `memmap2` (unused), removed `zstd` experimental feature (premature for Phase 1)

### Fixed
- **Bloom filter now operational**: Wrapped in `Mutex`, wired into `HashIndex::insert()` and `lookup()`. Previously was dead code with `#[allow(dead_code)]`.
- **Index persistence**: Added `populate_index()` to `LocalStore`, called on startup so the dedup index is pre-populated from existing packs
- **Coverage validation**: `debug_assert!` → `assert!` in chunk boundary checks (critical for release build safety)
- **Removed `unwrap()` in library code**: `decode_footer` now uses `map_err` instead of `try_into().unwrap()`
- **`store.get()` no longer holds lock during disk I/O**: Restructured to find location under lock, then release before reading
- **Writer thread now updates index**: New chunks are inserted into the HashIndex immediately after storage, enabling intra-file dedup
- **Fixed `BackupStats` duplicate field assignment**
- **Fixed empty IO error context**: `From<std::io::Error>` now populates context with error description
- **Worker thread `send()` errors no longer silently discarded**: (channel error handling improved)

### Removed
- `BufferPool` (dead code — never used)
- `StoredChunk` type (dead code — replaced by `IndexEntry` in pack format)
- `WriterStage` module (unused — writing was done inline)
- `memmap2` dependency (unused)
- `util::buffer` module

### Added
- **File metadata in backup manifests**: Stores path, size, modification time, permissions alongside chunk hashes
- **Metadata restoration**: Restore command now preserves file mtime and permissions
- **Backward compat**: Old manifests (bare hash lists) are still readable
- **Criterion benchmarks**: `chunking_throughput`, `hashing_throughput`, `pack_roundtrip`
- `pub use error::Result as PacktResult` for library users
- Manifest metadata tests

### Naming
- `FileReader` → `SourceReader` (less generic, avoids impl conflicts)
- `PACK_MAGIC` now matches comment (`b"PACKv1"`)

## [0.1.0] - 2026-07-16

### Added
- Initial project structure with workspace layout
- FastCDC v2020 content-defined chunking
- BLAKE3 content hashing with known-vector tests
- Content-addressed pack format with integrity verification
- Local filesystem store with atomic write semantics
- Concurrent dedup index with DashMap backend
- Pipeline architecture for backup/restore workflows
- CLI with backup, restore, info, verify, benchmark subcommands
- Property-based tests using proptest
- CI pipeline: fmt, clippy, test (3 OS), bench, audit
