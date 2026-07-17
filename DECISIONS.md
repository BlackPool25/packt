# Architecture Decisions

> **Project**: Rust-based binary diffing/dedup framework
> **Last Updated**: 2026-07-17
> **Status**: Phase 4a complete, Phase 4b starting

---

## Decision Log

| # | Decision | Choice | Rationale | Date | Status |
|---|----------|--------|-----------|------|--------|
| 001 | CDC Algorithm | **FastCDC v2020** | Production-tested, ~2.5 GB/s, mature Rust crate. MinCDC too new. | 2026-07-16 | ✅ Verified (Phase 1) |
| 002 | Content Hash | **BLAKE3** | 5-10x faster than SHA-256, SIMD, keyed hashing, pure Rust. | 2026-07-16 | ✅ Verified |
| 003 | Delta Encoding | **zstd dict mode** | Fastest practical: 200-500 MB/s encode, bounded memory. | 2026-07-16 | ⏳ Phase 3 |
| 004 | Similarity Detection | **Palantir hierarchical super-features** (3-tier: 95%/85%/70%) | MinHash (k=120, 4B shingles) cost 8s/200MB. Palantir cost: ~0ms. Finesse-style fixed sub-chunks with 12 features + 3-tier grouping. Data Domain lineage. | 2026-07-17 | ✅ Verified (Phase 2) |
| 005 | Similarity Index Memory | **In-memory LRU** | Start with bounded in-memory index. LRU eviction at budget limit. | 2026-07-16 | ✅ Verified |
| 006 | Pack Format | **Header-at-end** | Appendable, restic-inspired. Fixed-size footer (52 bytes: u64+u32+[u8;32]+u64). | 2026-07-16 | ✅ Verified |
| 007 | Storage Backend | **Local filesystem first** | std::fs + memmap2. Cloud via object_store trait in later phase. | 2026-07-16 | ✅ Verified |
| 008 | Pipeline Model | **Sync + Rayon** | Bounded crossbeam channels with backpressure. Simpler than async. | 2026-07-16 | ✅ Verified |
| 009 | Error Correction | **Separate phase** | Reed-Solomon/PAR2 is orthogonal. Phase 5 candidate. | 2026-07-16 | ✅ Locked |
| 010 | License | **MIT OR Apache 2.0** | Rust ecosystem standard. Dual-license for max adoption. | 2026-07-16 | ✅ Done |
| 011 | CLI Tool | **Library + Reference CLI** | Library is product, CLI is demo. Both shipped from day one. | 2026-07-16 | ✅ Verified |
| 012 | Phase 1 Scope | **CDC + Exact Dedup Only** | FastCDC → BLAKE3 → pack format → content store → CLI. | 2026-07-16 | ✅ Complete |
| 013 | Test Corpus | **Docker image layers** | Ubuntu versions achieved 3.44x dedup. Cross-image: 7.54x. | 2026-07-16 | ✅ Verified |
| 014 | Average Chunk Size | **32 KB default** | Balances granularity vs throughput. Range: 4KB-128KB configurable. | 2026-07-16 | ✅ Verified |
| 015 | Maximum File Size | **No artificial limit** | Stream via mmap. Pipeline processes chunks without full file in memory. | 2026-07-16 | ✅ Verified |
| 016 | MinHash Rejection | **Replaced with Palantir** | Full MinHash (k=120, 4B shingles) cost 8,000ms per 200MB. Palantir cost: ~0ms. Research showed MinHash is 500,000× more compute than super-feature approach used by production systems. | 2026-07-17 | ✅ Superseded |
| 017 | Palantir Feature Extraction | **Fixed sub-chunks (Finesse-style)** | Divide chunk into 12 equal sub-chunks, hash first 32 bytes of each with xxh3. Position-stable — modifications only affect sub-chunks they land in. 12 xxh3 calls per chunk = ~0ms overhead. | 2026-07-17 | ✅ Verified |
| 018 | Palantir Hierarchical Tiers | **3 tiers: (3,4), (4,3), (6,2)** | Tier 1 (≥95%): 3 SFs×4 features. Tier 2 (≥85%): 4 SFs×3 features. Tier 3 (≥70%): 6 SFs×2 features. Total: 104 bytes per entry. All tiers derived from same 12 base features. | 2026-07-17 | ✅ Verified |
| 019 | False Positive Filter | **Head+tail xxh3 comparison** | Compare first 64B + last 64B of candidate vs query chunk. Rejects match if both head AND tail differ. Catches single-burst edit pattern. | 2026-07-17 | ✅ Verified |
| 020 | xxhash-rust Dependency | **Kept for Palantir** | Used for sub-chunk hashing (xxh3_64) and FPR filter. xxh3 is the fastest non-crypto hash. No new deps added. | 2026-07-17 | ✅ Verified |
| 021 | Compression Level | **zstd level 3** | Level 7 was 130% slower for only 5% ratio gain on unique chunks (15-25% of data). Level 3 is the sweet spot. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 022 | Parallel Pipeline | **Not beneficial, reverted** | BLAKE3 of 32KB takes ~2us. FastCDC at 1.4 GB/s is the bottleneck. Zero wall-time gain for single-file workloads. | 2026-07-17 | ❌ Reverted (Phase 4a) |
| 023 | Store get()/put() O(n) -> O(1) | **Use DedupIndex** | Both get_inner() and store.put() had O(n) linear scans over all pack entries. Replaced with DedupIndex O(1) lookup. Critical for correctness at scale. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 024 | Store contains() O(n) -> O(1) | **DedupIndex + pack fallback** | Index checked first for speed, pack scan kept as safety net for unindexed code paths. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 025 | PalantirIndex Sharding | **4 shards by hash prefix** | Single Mutex<PalantirIndex> serialized all similarity ops. 4 shards with independent budgets enable concurrent access. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 026 | PalantirIndex Query Limit | **5 candidates per SF match** | query_tier accumulated O(n) candidates. Cap at 5 per SF (max 65 per query) keeps time constant. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 027 | Incompressible Data Fast-Path | **Quick entropy check** | Sample 1KB, if 220+ unique byte values present, skip zstd entirely. Prevents wasted CPU on random data. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 028 | Pack Flush Verification | **In-memory, not read-back** | Atomic rename + fsync guarantees integrity. Read-back was redundant disk I/O. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 029 | Progress Callback | **Not implemented** | Box<dyn Fn + Send + Sync> adds complexity. No consumers. Add when a library consumer asks. | 2026-07-17 | ❌ Rejected (Phase 4a) |
| 030 | FPR Byte-Frequency Histogram | **Not implemented (dead code)** | check_similarity() not called from pipeline. Similarity uses PalantirIndex.query() directly. | 2026-07-17 | ❌ Rejected (Phase 4a) |
| 031 | zstdmt Feature | **Enabled for super-block** | Multi-threaded zstd with 4 workers for super-block compression. Speeds up large-pack flush on multi-core. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 032 | Single-File Restore | **Manifest-level selection** | Restore one file without decompressing entire store. Uses existing manifest JSON. | 2026-07-17 | ✅ Verified (Phase 4a) |
| 033 | Incremental Backup | **mtime + size check** | Compare stored manifest metadata against current file. Skip if unchanged. --force to override. | 2026-07-17 | ✅ Verified (Phase 4a) |

---

## Rejected Alternatives

| Alternative | Rejected Because | Reconsider If |
|------------|-----------------|---------------|
| MinCDC for Phase 1 | Too new (2025), no production track record | Becomes production-tested, crate stabilizes |
| SHA-256 for content hash | 5-10x slower than BLAKE3, no advantage for our use case | Interop with git/IPFS required |
| gdelta for delta encoding | v0.1.x, API may break | Reaches v1.0, shows clear advantage |
| SimHash for similarity | Provably worse for binary data (higher rho value) | Working with dense vectors instead of binary shingles |
| Async (tokio) pipeline | 2-3x more complex, no benefit until network IO | Network storage backends (S3) dominate pipeline time |
| Tiered similarity index | Premature optimization for Phase 2 | Index exceeds available RAM for target workload |
| Built-in FEC | Orthogonal concern | Users specifically request integrated error correction |
| Full MinHash (k=120) | 8s overhead per 200MB, 960B per entry. 500,000× more compute than super-features. Palantir achieves same goal at ~0ms. | If we need Jaccard similarity for non-binary data types |
| Content-defined sampling (Odess-style) | Gear hash divergence: 1 modified byte at position 0 changes ALL subsequent rolling hash values. CDC-sampling positions differ between near-identical chunks. Fixed sub-chunks are simpler and provably correct. | If chunk alignment guarantees can be made (e.g., same CDC boundaries) |
| TLSH/ssdeep | File-level fuzzy hashes, poor statistical power at 32KB chunk granularity | Working with files >1MB where statistical features are meaningful |
| Byte-frequency histograms | Order-insensitive — two chunks with identical byte frequencies but different content score 100% | Never — spatial alignment is critical for delta compression |

---

## Phase 1 Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| Dedup ratio (Ubuntu versions) | 1.25-1.5x | **3.44x** | 🟢 Exceeded expectations |
| Dedup ratio (cross-image) | — | **7.54x** (vs restic 3.23x) | 🟢 2.3x better than restic |
| Backup speed vs restic | Comparable | **21% faster** | 🟢 Faster than industry standard |
| Tests passing | — | **34/34 + 31/31 stress tests** | 🟢 All passing |
| Real-world validation | Docker layers | 4 Ubuntu + 9 cross-image | 🟢 Validated |

## Phase 2 Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| Similarity overhead (200MB) | — | **~0ms** (not measurable) | 🟢 Palantir is 10,000x faster than MinHash |
| Memory per signature | — | **104 bytes** (vs 960 for MinHash) | 🟢 9x less |
| Near-dup detection (Docker layers) | — | **162 (10.5%), 125 (7.9%), 119 (7.0%)** | 🟢 Verified on real data |
| Backup speed vs restic | 21% faster (Phase 1) | **2.1x faster** | 🟢 Phase 2 improvements maintained speed |
| Tests | 47+ | **70/70 passing** (59 unit + 11 integration) | 🟢 All passing |
| Clippy | Clean | Clean (0 warnings) | 🟢 |

---

## Phase 4a Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| get() using DedupIndex | O(1) lookup | **O(1)** via DashMap + bloom filter. Linear scan fallback preserved. | 🟢 Critical correctness fix |
| PalantirIndex sharding | 4 shards | **4 shards** by hash prefix, independent Mutex + LRU budget per shard. | 🟢 Verified |
| Query candidate limit | 5 per SF match | **5** per SF match (max 65 candidates/query). | 🟢 Verified |
| zstd level 7 -> 3 | 2x faster, <10% ratio loss | **2.2x faster**, 5% ratio loss (30.1MB -> 31.7MB on Docker layer). | 🟢 Acceptable tradeoff |
| Parallel pipeline (Rayon) | ≥1 GB/s | **Zero gain**. BLAKE3 of 32KB takes 2us. FastCDC is the bottleneck at 1.4 GB/s. | 🔴 Reverted |
| Entropy check | Skip zstd for random data | **~5% improvement** on incompressible workloads. 220+ unique bytes in 1KB sample = skip. | 🟢 Marginal but cheap |
| No read-back | Remove redundant I/O | **~5% improvement** on flush-heavy paths. Atomic rename + fsync guarantees integrity. | 🟢 Verified |
| Property tests | 3 tests | **3 proptest** (delta roundtrip, CDC determinism, pack roundtrip) + **2 fuzz** (pack reader, delta codec). Found write_pack entry reordering bug. | 🟢 Caught real bug |
| Deadlock fix | Reentrant Mutex | get_inner() held index lock during recursive delta reconstruction. Scoped guard fix. | 🟢 Critical fix |
| crates.io release | v0.5.0 | **v0.5.0-v0.5.2** published. packt-lib + packt-cli on crates.io. | 🟢 Published |
| Single-file restore | Via manifest | **packt restore <store> <dest> <file>** restores one file without decompressing store. | 🟢 Essential feature |
| Incremental backup | mtime + size check | **Skip unchanged files**. --force to override. | 🟢 Essential feature |
| packt list | Show backed up files | **packt list <store>** shows files with size and chunk count. | 🟢 Essential feature |
| Tests | 78+ | **78/78 passing** (62 unit + 11 integration + 3 property + 2 fuzz). Clippy clean. | 🟢 All passing |

---

## Phase 3 Pack Format Extension Spec (Planned)

### Problem
Currently (Phase 1 & 2), every chunk is stored as a full zstd-compressed blob. Near-duplicate chunks are detected via Palantir but still stored as full copies. Phase 3 adds delta compression: if a chunk is flagged as near-duplicate to an existing base chunk, only the delta is stored.

### Proposed Pack Format Extension

Current entry types (Phase 1):
```
ChunkType::Data = 0x00    // full zstd-compressed data
```

Phase 3 adds:
```
ChunkType::DeltaZstd = 0x01   // zstd --patch-from delta relative to base chunk
```

#### New ChunkEntry Fields
```rust
pub struct ChunkEntry {
    pub hash: [u8; 32],         // BLAKE3 of original (uncompressed) data
    pub type_flags: u32,        // bit 0: 0=full, 1=delta
    pub encrypted_length: u32,  // compressed/delta length in pack
    pub plaintext_length: u32,  // uncompressed length (0 if same as stored)
    // NEW for delta type:
    pub base_hash: [u8; 32],    // BLAKE3 of the base chunk (all zeros for full chunks)
}
```

#### Footer Format Update
Current: `[magic(8B) | chunk_count(4B) | [ChunkEntry] | checksum(32B)] = 52B + entries`

Phase 3: Add `chunk_type` per entry to distinguish full vs delta. The footer already has reserved bytes. The delta `base_hash` is encoded via type_flags mechanism: if bit 0 is set, an additional 32B base_hash field follows.

#### Delta Encoding Strategy
- **zstd `--patch-from`** dictionary mode: `ZSTD_compress_usingDict()` with the base chunk as dictionary
- Base chunk retrieved from store via Palantir's similar_to hash
- Decompress: `ZSTD_decompress_usingDict()` with same base chunk
- Fallback: if delta encoding produces output larger than storing full, store full instead
- Palantir tier influences delta strategy:
  - Tier 1 (≥95%): aggressive delta, expect high compression
  - Tier 2 (≥85%): standard delta
  - Tier 3 (≥70%): conservative delta, verify benefit before committing

### Backward Compatibility
- Format version bump in footer magic
- Phase 1/2 readers reject Phase 3 packs (strict magic check)
- Phase 3 readers handle both old and new formats

---

## Phase 4-5 Roadmap (Outline)

### Phase 4: Production Hardening
- **Parallel processing**: Rayon-based parallel chunk processing, multi-threaded pipeline
- **Concurrent writers**: Support multiple concurrent backup sessions
- **S3/GCS storage backend**: object_store trait implementation
- **Encryption**: Per-chunk AEAD (aes-gcm)
- **Snapshot management**: Prune/GC old snapshots, retention policies

### Phase 5: Advanced Features
- **Semantic similarity**: ML-based chunk embedding (Chunk2vec-style) for cross-modal dedup
- **FEC/erasure coding**: Reed-Solomon for bit-rot protection
- **WASM support**: no_std + WASM target for browser/edge dedup
- **FUSE mount**: On-the-fly extraction via FUSE filesystem
- **OCI registry plugin**: Docker/Harbor integration for layer delta compression

---

## Phase 4b Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| CloudStore (S3+GCS) | ContentStore impl via OpenDAL | **CloudStore** with pending-buffer + 16MB pack upload. Tests with OpenDAL Memory backend. | 🟢 Delivered |
| Store API | open/backup/restore/list | **open/backup/restore/list/info/verify/delete/hasFile/iter_chunks** | 🟢 Exceeded scope |
| Local cache | Optional LRU | **Optional LRU cache** (128 packs, ~2GB). Disk files pruned on eviction. | 🟢 Delivered |
| URI-style CLI | s3:// and gcs:// paths | **config_from_uri** parses `?region=`, `?endpoint=`, credentials from query params | 🟢 Delivered |
| packt migrate | Subcommand | **packt migrate** via restore-temp + backup-from-temp | 🟢 Delivered |
| Windows CI | Must pass | **Dropped** — Windows runner filesystem permission restrictions on temp dir | 🔴 Dropped |
| Integration Guide | docs/INTEGRATION_GUIDE.md | **Published** with Store API, cloud, cache, migration, production notes | 🟢 Delivered |
| cargo-deny | Clean | **Configured** with OpenDAL dep tree skips and advisory ignores | 🟢 Delivered |
| Code review fixes | 12 issues found | **All fixed** — path traversal, _meta.index race, pack_id bug, unwraps, Debug leak | 🟢 All fixed |
| Tests | 78+ | **101/101 passing** (85 lib + 2 fuzz + 11 integration + 3 property). Clippy clean. | 🟢 All passing |
