# Learning & Progress Journal

> **Purpose**: Track mistakes, lessons, research findings, and progress.
> **Rule**: Read this before starting any new task. Add entries when bugs/regressions/design mistakes occur.
> **Last Updated**: 2026-07-16

---

## Active Project State

**Current Phase**: Phase 2 (Similarity Detection)
**Phase 1 Status**: ✅ Complete + Production Hardening (Jul 16, 2026)
**Project**: packt — https://github.com/BlackPool25/packt
**Last Successful Build**: Release — 0 warnings, 0 errors
**Test Status**: 42/42 passing (34 unit + 8 integration) + 5 benchmarks
**Clippy**: Clean (0 warnings, -D warnings)
**CI**: All jobs pass (fmt, clippy, test on 3 OS, bench, audit, build-all)
**Git Workflow**: PRs only — never push directly to main
**Real-world validated**: 12.75x cross-image dedup (9 Docker images), 2.2x faster than restic

---

## Lessons Learned

### [2026-07-16] Crate Naming Matters — "compressor" Implies gzip, Not Dedup
**Category**: Design
**Component**: Project naming
**Finding**: The name "compressor" confused users expecting a compression library. The core value is deduplication.
**Fix**: Renamed to `dedup-lib` / `dedup-cli` with binary name `dedup`.
**Prevention**: Choose crate names that reflect the core differentiator, not a generic capability.

### [2026-07-16] bincode v2 Project Shut Down — Pack Format at Risk
**Category**: Dependency
**Component**: store/pack.rs
**Symptom**: bincode crates.io version started emitting compiler errors; project unmaintained since late 2025.
**Root Cause**: bincode v2 development ceased permanently. Any pack file written depends on a dead serialization format.
**Fix**: Migrated to `postcard` (maintained, same compact binary format, active community).
**Prevention**: Regularly audit critical dependencies for maintenance status. Don't assume v2 means "stable."

### [2026-07-16] Bloom Filter Was Dead Code — Never Populated
**Category**: Bug
**Component**: index/hashindex.rs
**Symptom**: `#[allow(dead_code)]` on bloom_filter revealed it was never used. `insert()` was never called.
**Root Cause**: Bloom filter field added but never wired into the insert/lookup path. Thread safety issue (required `&mut self` behind `&self` API).
**Fix**: Wrapped in `Mutex`, called `insert()` from `DedupIndex::insert()`, checked in `lookup()` for quick negative.
**Prevention**: When adding performance infrastructure, test that it's actually called. An allow(dead_code) annotation is a red flag.

### [2026-07-16] `debug_assert!` Is Not Enough for Correctness-Critical Checks
**Category**: Bug
**Component**: chunking/fastcdc.rs
**Symptom**: Gap/overlap coverage validation was compiled out in release builds. Undetected chunk boundary gaps could cause silent corruption.
**Fix**: Changed to `assert!` (always-on).
**Prevention**: Use `assert!` for correctness invariants that must hold in production. Reserve `debug_assert!` for expensive checks only.

### [2026-07-16] Ephemeral Index Breaks Cross-Session Dedup
**Category**: Design
**Component**: pipeline + index
**Symptom**: Each CLI invocation created a fresh empty index. All previous chunks appeared "new," causing O(n) rescans and re-storage.
**Fix**: Added `populate_index()` to LocalStore; called on startup to pre-load index from existing packs. Writer thread now updates index after each store.
**Prevention**: For store-like abstractions, the index should be populated from durable state on construction, not start empty.

### [2026-07-16] Pack Format Serializer Choice Blocks Migrations
**Category**: Research
**Component**: store/pack.rs
**Finding**: bincode v2 became defunct, requiring a full serialization format migration. Using postcard with `use-std` feature for `to_stdvec`.
**Impact**: Postcard produces same compact binary layout but API is slightly different (`postcard::take_from_bytes` vs `bincode::serde::decode_from_slice`).

### [2026-07-16] Mutex Held During Disk I/O Serializes Reads
**Category**: Performance
**Component**: store/local.rs
**Symptom**: `store.get()` held the `Mutex<StoreState>` lock while doing `std::fs::read()` and `pack::read_chunk()`.
**Fix**: Restructured to extract location under lock, drop lock, then read from disk.
**Impact**: Concurrent reads can now overlap, critical for multi-file restore performance.

---

## Bug Tracking

| # | Date | Component | Severity | Status | Summary |
|---|------|-----------|----------|--------|---------|
| 001 | 2026-07-16 | store/pack.rs | High | Fixed | Footer size non-deterministic with bincode v2 |
| 002 | 2026-07-16 | index/hashindex.rs | Med | Fixed | Orphaned code from incomplete edit |
| 003 | 2026-07-16 | pipeline/mod.rs | Low | Fixed | Unused variable warning |
| 004 | 2026-07-16 | types.rs | Low | Fixed | u8 vs char mismatch in hex_encode |
| 005 | 2026-07-16 | store/local.rs | Low | Fixed | Unused imports |
| 006 | 2026-07-16 | index/hashindex.rs | High | Fixed | Bloom filter dead code — never populated |
| 007 | 2026-07-16 | chunking/fastcdc.rs | High | Fixed | debug_assert! in release = silent corruption risk |
| 008 | 2026-07-16 | store/pack.rs | High | Fixed | unwrap() in library decode_footer |
| 009 | 2026-07-16 | error.rs | Low | Fixed | Empty IO error context string |
| 010 | 2026-07-16 | pipeline/mod.rs | Med | Fixed | Index not updated during pipeline — no intra-file dedup |

---

## Research Findings

| # | Date | Topic | Finding | Impact |
|---|------|-------|---------|--------|
| R09 | 2026-07-16 | compressor vs restic | compressor 7.54x vs restic 3.23x cross-image dedup | FastCDC 32KB beats Rabin 1MB on granularity |
| R10 | 2026-07-16 | compressor speed | 21% faster than restic on 200MB random data | Rust zero-cost abstractions + no GC |
| R11 | 2026-07-16 | bincode status | bincode v2 project permanently shut down (late 2025) | Migrated to postcard |
| R12 | 2026-07-16 | postcard 1.x API | `to_stdvec` requires `use-std` feature, not `alloc` | postcard = { features = ["use-std"] } |

---

## Phase 1 Verdict (Post-Hardening)

```
Storage integrity:    100% verified (all chunks pass BLAKE3 hash check)
No data loss:         Confirmed — every stored chunk matches its hash
Cross-version dedup:  3.44x on Ubuntu 22.04->24.04
Cross-image dedup:    7.54x (2.3x better than restic)
Backup speed:         21% faster than restic
Bloom filter:         Operational (was dead code before)
Index persistence:    Pre-populated from existing packs on open
Coverage validation:  Always-on (was debug-only before)
Tests:                42/42 passing (34 unit + 8 integration)
Benchmarks:           5 criterion benchmarks
Clippy:               Clean (-D warnings)
Dependencies:         bincode replaced with postcard (bincode is defunct)
```

---

## Phases Completed

| Phase | Completed | Duration | Key Learnings |
|-------|-----------|----------|---------------|
| Phase 1 | 2026-07-16 | 1 day | FastCDC confirmed; Pack footer must use fixed-size encoding; Docker OCI format requires blob-level extraction; AtomicU32 for shared state; All 31 tests pass with real-world validation at 3.44-7.54x dedup ratios |
| Phase 1 Hardening | 2026-07-16 | ~2 hours | Bloom filter was dead code; bincode v2 defunct; debug_assert insufficient; index not persisted; Mutex held during IO; crate naming matters |
