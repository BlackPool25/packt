# Changelog

## [0.2.0] - 2026-07-16

### Changed
- **Crate rename**: `compressor-lib` → `dedup-lib`, `compressor-cli` → `dedup-cli`, binary `compressor` → `dedup`
- **Error type rename**: `CompressionError` → `DedupError`
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
- `pub use error::Result as DedupResult` for library users
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
