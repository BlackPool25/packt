# Architecture Decisions

> **Project**: Rust-based binary diffing/dedup framework
> **Last Updated**: 2026-07-16
> **Status**: Phase 1 complete, Phase 2 starting

---

## Decision Log

| # | Decision | Choice | Rationale | Date | Status |
|---|----------|--------|-----------|------|--------|
| 001 | CDC Algorithm | **FastCDC v2020** | Production-tested, ~2.5 GB/s, mature Rust crate. MinCDC too new. | 2026-07-16 | ✅ Verified (Phase 1) |
| 002 | Content Hash | **BLAKE3** | 5-10x faster than SHA-256, SIMD, keyed hashing, pure Rust. | 2026-07-16 | ✅ Verified |
| 003 | Delta Encoding | **zstd dict mode** | Fastest practical: 200-500 MB/s encode, bounded memory. | 2026-07-16 | ⏳ Phase 3 |
| 004 | Similarity Detection | **MinHash + LSH banding** | MinHash provably outperforms SimHash for binary data. b=20, r=6 gives 0.96 recall at s=0.8. | 2026-07-16 | ⬅️ Phase 2 (NOW) |
| 005 | Similarity Index Memory | **In-memory LRU** | Start with bounded in-memory index. LRU eviction at budget limit. | 2026-07-16 | ⬅️ Phase 2 |
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

---

## Phase 1 Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| Dedup ratio (Ubuntu versions) | 1.25-1.5x | **3.44x** | 🟢 Exceeded expectations |
| Dedup ratio (cross-image) | — | **7.54x** (vs restic 3.23x) | 🟢 2.3x better than restic |
| Backup speed vs restic | Comparable | **21% faster** | 🟢 Faster than industry standard |
| Tests passing | — | **34/34 + 31/31 stress tests** | 🟢 All passing |
| Real-world validation | Docker layers | 4 Ubuntu + 9 cross-image | 🟢 Validated |
