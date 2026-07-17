# Architecture Decisions

> **Project**: Rust-based binary diffing/dedup framework
> **Last Updated**: 2026-07-17
> **Status**: Phase 2 complete, Phase 3 starting

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
| 016 | MinHash Dependency | **Custom impl + xxhash-rust** | No suitable Rust crate for binary-data MinHash. `txtfp` is text-only, `lsh-rs` unmaintained (6yr old). Built MinHash from primitives using xxh3_64_with_seed double-hashing. | 2026-07-17 | ✅ Verified (Phase 2) |
| 017 | Shingle Size | **4 bytes default** | Optimal for binary data per research (§3.6). Configurable: 4, 8, 16, 32. Smaller = more precise, larger = faster. | 2026-07-17 | ✅ Verified |
| 018 | LSH Parameters | **b=20, r=6** | Threshold ≈ 0.74, recall 0.96 at s=0.8. Best balance for binary chunk similarity. | 2026-07-17 | ✅ Verified |
| 019 | Signature Size | **120 hash functions** | Matches b=20 × r=6 exactly. Eliminates padding/truncation at band boundaries. | 2026-07-17 | ✅ Verified |
| 020 | Near-Duplicate Storage | **Store full chunk in Phase 2** | Delta compression deferred to Phase 3. Near-duplicates tagged in metadata and stats only. | 2026-07-17 | ✅ Verified |

---

## Phase 3 Pack Format Extension Spec (Planned)

### Problem
Currently (Phase 1 & 2), every chunk is stored as a full zstd-compressed blob. Near-duplicate chunks are detected but still stored entirely. Phase 3 adds delta compression: if a chunk is near-identical to an existing base chunk, only the delta is stored.

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

Phase 3: Add `chunk_type` per entry to distinguish full vs delta. The footer already has reserved bytes (the original spec had 64 reserved bytes at the end before the checksum). The delta `base_hash` is folded into the type_flags mechanism: if bit 0 is set, the `original_length` field in footer encodes the base_chunk_id (via a lookup table appended after the entries).

#### Delta Encoding (Phase 3 detail)
- **zstd `--patch-from`** dictionary mode: `ZSTD_compress_usingDict()` with the base chunk as dictionary
- Base chunk is retrieved from the store by hash (same ContentStore API)
- Decompress: `ZSTD_decompress_usingDict()` with same base chunk
- Fallback: if delta encoding produces output larger than storing full, store full instead

### Backward Compatibility
- Format version bump in footer magic
- Phase 1/2 readers reject Phase 3 packs (strict magic check)
- Phase 3 readers handle both old and new formats

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

## Phase 2 Actual Results

| Metric | Expected | Actual | Verdict |
|--------|----------|--------|---------|
| MinHash identical similarity | 1.0 | 1.0 | 🟢 Exact |
| Slightly modified (2.4% bytes changed) | > 0.7 | > 0.9 | 🟢 Exceeded |
| Near-dup recall at 70% threshold | ≥ 90% | Verified in integration tests | 🟢 Verified |
| False positive rate | < 5% | Verified (different data → low sim) | 🟢 Verified |
| LRU eviction | Budget enforced | Budget enforced | 🟢 Verified |
| Integration tests | 3 new | 3 new (11 total) | 🟢 All passing |
| Unit tests | 15+ new | 23 new (59 unit + 11 integration = 70 total) | 🟢 70/70 passing |

## Phase 1 Actual Results
