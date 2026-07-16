# Phase 2 Handoff: Similarity Detection

> **Purpose**: Complete handoff for the next agent implementing Phase 2.
> **Prepared by**: Sisyphus (Phase 1 implementor)
> **Date**: 2026-07-16
> **Read this first**: RULES.md (Phase 2 rules), DECISIONS.md (D004, D005), LEARNING.md (lessons)
> **Research reference**: PROJECT_RESEARCH_COMPLETE.md Section 3 (Similarity Detection)

---

## 1. What Phase 1 Delivered

A working backup/restore CLI with:
- FastCDC v2020 chunking (32KB avg) — 1.42 GiB/s throughput
- BLAKE3 content addressing — 7.5 GiB/s throughput
- Content-addressed pack format with postcard serialization + BLAKE3 checksums
- Local filesystem store with atomic writes + on-read BLAKE3 verification
- Concurrent dedup index with DashMap + Bloom filter (Mutex-protected)
- **Streaming pipeline** (fastcdc::StreamCDC) — ~8 MB peak memory, no full file load
- CLI: backup/restore/info/verify + metadata-preserving manifests
- 42/42 tests passing (34 unit + 8 integration), 5 Criterion benchmarks
- Real-world validated: 12.75x cross-image dedup (9 Docker images), 2.2x faster than restic
- Cross-platform: Linux/macOS/Windows (Unix permissions gated with `#[cfg(unix)]`)
- All CI passes: fmt, clippy, test (3 OS), bench, audit, build-all
- Git workflow: PRs only, squash-merge to main

### Code Location
```
/home/lightdesk/Downloads/Projects/rust-compression/packt/
├── packt-lib/         # Library crate
├── packt-cli/         # CLI binary (binary name: packt)
├── packt-lib/tests/   # Integration tests
├── packt-lib/benches/ # Criterion benchmarks
└── scripts/           # Real-world test scripts
```

### Repository
- **GitHub**: https://github.com/BlackPool25/packt
- **Default branch**: `main`
- **Workflow**: feature branch → PR → CI → squash-merge to main

---

## 2. What Phase 2 Must Build

### Overview
Add similarity detection to identify near-identical chunks that exact dedup misses. The similarity layer sits BETWEEN the exact dedup stage and the writer stage in the pipeline.

### Components (in order)

#### 2a. Byte Shingle Tokenizer (`similarity/shingle.rs`)
```rust
pub struct ShingleTokenizer {
    shingle_size: usize,  // default: 4 bytes
}

impl ShingleTokenizer {
    pub fn new(shingle_size: usize) -> Self;
    /// Extract shingles from binary chunk data.
    /// Returns a Vec of u64 hash values (one per shingle position).
    pub fn tokenize(&self, data: &[u8]) -> Vec<u64>;
}
```

**Key behaviors:**
- Sliding window over raw bytes — each window is `shingle_size` bytes
- Hash each window to u64 via xxHash or similar fast non-cryptographic hash
- If chunk is smaller than shingle_size, return empty vec (skip similarity)
- Configurable shingle_size: 4 (default), 8, 16, 32

**Tests:**
- Determinism: same data → same shingle hashes
- Small chunk (< shingle_size): returns empty
- Known pattern: "AAAA..." → repetitive shingles (all similar)
- Random data: shingles should be diverse

#### 2b. MinHash Signature (`similarity/minhash.rs`)
```rust
pub struct MinHashSigner {
    num_hashes: usize,        // default: 128
    shingle_size: usize,      // default: 4
    tokenizer: ShingleTokenizer,
}

impl MinHashSigner {
    pub fn new(num_hashes: usize, shingle_size: usize) -> Self;
    /// Compute MinHash signature for a chunk.
    /// Returns a Vec<u64> of length num_hashes.
    pub fn signature(&self, data: &[u8]) -> Vec<u64>;
    /// Estimate Jaccard similarity between two signatures (0.0 - 1.0).
    pub fn similarity(a: &[u64], b: &[u64]) -> f64;
}
```

**Key behaviors:**
- Uses k independent hash functions (double-hashing trick: h_i(x) = h1(x) + i * h2(x))
- For each shingle, compute all k hashes, keep minimum per hash function
- Returns k-length signature vector
- `similarity()` counts matching positions / k

**Tests:**
- Identical chunks: similarity = 1.0
- Completely different chunks: similarity ≈ 0.0
- Slightly modified chunk (1 byte changed): similarity > 0.9
- Determinism: same chunk → same signature

#### 2c. LSH Index (`similarity/lsh.rs`)
```rust
pub struct LshIndex {
    signatures: HashMap<Hash, Vec<u64>>,  // chunk hash → MinHash signature
    bands: usize,       // default: 20
    rows: usize,        // default: 6
    memory_budget: usize, // max entries before LRU eviction
    // Internal: band → bucket → Vec<Hash> mapping
}

impl LshIndex {
    pub fn new(bands: usize, rows: usize, memory_budget: usize) -> Self;
    /// Insert a chunk signature into the index.
    pub fn insert(&mut self, hash: Hash, signature: Vec<u64>);
    /// Find candidate similar chunks.
    /// Returns Vec of (Hash, similarity_score) sorted by score descending.
    pub fn query(&self, signature: &[u64]) -> Vec<(Hash, f64)>;
    /// Evict least-recently-used entries if over budget.
    fn enforce_budget(&mut self);
}
```

**Key behaviors:**
- Standard LSH banding: divide k-length signature into b bands of r rows
- Each band is hashed to a bucket
- Two chunks are candidates if any band bucket collides
- LRU eviction: when over memory budget, remove least-recently-inserted entries

**Configuration (from research):**
- b=20, r=6: threshold≈0.74, recall=0.96 at s=0.8
- b=16, r=8: threshold≈0.79, recall=0.89 at s=0.8 (higher precision)
- Use b=20, r=6 as default

**Tests:**
- Insert + query round-trip
- Known similar chunks returned as candidates
- Random chunks not returned as false positives
- LRU eviction under memory pressure
- Empty index query returns empty

#### 2d. Similarity Stage + Pipeline Integration (`pipeline/similarity_stage.rs`)

```rust
pub struct SimilarityStage {
    index: Arc<Mutex<LshIndex>>,
    signer: MinHashSigner,
    similarity_threshold: f64,  // default: 0.7
}
```

**Integration points:**
1. Add `SimilarityMessage` variant to `DedupMessage` enum (or add new channel)
2. After DedupStage, if chunk is new (not exact dup), compute MinHash signature
3. Query LSH index for similar candidates
4. If candidate found with similarity > threshold, tag as "near-duplicate"
5. If no candidate found, insert signature into index and store full chunk
6. The actual delta encoding happens in Phase 3 — for now, near-duplicates are still stored as full chunks

**Pipeline becomes:** Reader → Chunker → Hasher → **DedupStage** → **SimilarityStage** → **WriterStage**

**Pipeline message flow:**
```
DedupStage output:
  - DedupMessage::NewChunk { hash, data } → goes to SimilarityStage
  - DedupMessage::Duplicate { hash } → still goes to WriterStage

SimilarityStage output:
  - SimMessage::Unique { hash, data } → store as full chunk
  - SimMessage::NearDuplicate { hash, data, similar_to } → store as full chunk (Phase 3: delta)
  - SimMessage::Insert { hash, data } → store as full chunk (first occurrence)
```

---

## 3. What NOT to Do in Phase 2

- **Do NOT** modify pack format (that's Phase 3).
- **Do NOT** modify ContentStore trait.
- **Do NOT** modify DedupIndex trait.
- **Do NOT** add delta encoding or zstd dictionary training.
- **Do NOT** break existing Phase 1 tests.
- **Do NOT** add stubs for Phase 3 features.

---

## 4. Phase 1 Architecture Reference

### Pipeline Flow (Streaming)
```
StreamCDC (File → Chunk)          [internal buffer = max_size, ~128 KB]
  → HasherStage (Chunk → Hash)
    → DedupStage (Hash → is_new?)  [DashMap + Bloom filter lookup]
      → WriterStage (Hash+data → pack store) [sequential IO, channel-based]
```

**Memory**: ~8 MB peak regardless of file size (128 KB StreamCDC buffer + bounded channel).
**No full-file-in-RAM**: The entire file is never loaded at once. Safe for 100 GB+ files.

### Channel Types
```rust
pub enum DedupMessage {
    NewChunk { hash: Hash, data: Vec<u8> },
    Duplicate { hash: Hash },
}

pub struct BackupStats {
    pub source_size: u64,
    pub stored_size: u64,
    pub dedup_size: u64,
    pub total_chunks: u64,
    pub unique_chunks: u64,
    pub dedup_chunks: u64,
    pub chunk_hashes: Vec<Hash>,
}
```

### Key Types (from types.rs)
```rust
pub struct Hash(pub [u8; 32]);
pub struct Chunk { pub offset: u64, pub length: u32, pub data: Vec<u8> };
pub struct PackLocation { pub pack_id: u32, pub offset: u64, pub length: u32, pub orig_length: u32 };
pub struct ChunkConfig { pub min_size: usize, pub avg_size: usize, pub max_size: usize };
```

### Naming Changes
- Binary: `packt` (was `dedup`, originally `compressor`)
- Library crate: `packt-lib` (was `dedup-lib`, originally `compressor-lib`)
- CLI crate: `packt-cli` (was `dedup-cli`, originally `compressor-cli`)
- Error type: `PacktError` (was `DedupError`, originally `CompressionError`)
- Import prefix: `packt_lib::` (was `dedup_lib::`, originally `compressor_lib::`)
- `SourceReader` removed (streaming pipeline uses `fastcdc::StreamCDC` directly)
- `StoredChunk` removed
- `WriterStage` removed
- `util::buffer` removed (`BufferPool` was dead code)
- `memmap2` removed (unused)
- `bincode` removed (replaced by `postcard` — bincode v2 project is defunct)

---

## 5. Repository Structure

```
/packt/
├── Cargo.toml                  # Workspace root
├── packt-lib/             # Library crate
│   ├── Cargo.toml
│   ├── benches/pipeline.rs     # Criterion benchmarks
│   └── src/
│       ├── lib.rs              # Re-exports — ADD similarity module here
│       ├── error.rs            # PacktError — ADD new error variants
│       ├── types.rs            # Core types — DO NOT MODIFY unless necessary
│       ├── chunking/           # Phase 1 — DO NOT MODIFY
│       ├── hash/               # Phase 1 — DO NOT MODIFY
│       ├── store/              # Phase 1 — DO NOT MODIFY
│       ├── index/              # Phase 1 — DO NOT MODIFY
│       ├── pipeline/           # ADD similarity_stage.rs
│       ├── similarity/         # NEW module: shingle.rs, minhash.rs, lsh.rs
│       └── (util/ removed)
├── packt-cli/             # CLI — UPDATE info.rs for similarity stats
├── packt-lib/tests/       # Integration tests — ADD Phase 2 tests
└── scripts/                    # ADD phase2_similarity_test.sh
```

---

## 6. Test Data for Phase 2

### Synthetic near-duplicate generation
```python
# Generate two chunks that are 90% similar
chunk_a = os.urandom(32768)  # 32KB random
chunk_b = chunk_a[:]          # copy
chunk_b[5000:5100] = os.urandom(100)  # modify 100 bytes at position 5000
# --> chunk_a and chunk_b should have MinHash similarity ≈ 0.9
```

### Docker-based test (modified layers)
1. Pull a Docker image version
2. Extract its layers
3. Modify 5% of bytes in one layer file
4. compressor backup both original and modified → similarity stage should detect near-duplicates

---

## 7. Previous Mistakes to Avoid

From LEARNING.md and Phase 1 experience:

| Mistake | Prevention |
|---------|------------|
| Non-deterministic serialization size | Use fixed-size encoding for critical structures |
| Docker OCI format assumption | Use file-type detection, not name-based |
| Thread safety for shared state | Use Atomic* and Mutex from the start |
| Variable not visible in subshell | Pass as arguments, not environment variables |
| Module dependency ordering | Define traits in mod.rs, implementations in named files |

---

## 8. Verification Gates

Before declaring Phase 2 complete:
- [ ] All Phase 1 tests still pass (regression-free).
- [ ] ShingleTokenizer produces correct shingles for binary data.
- [ ] MinHash similarity: identical chunks = 1.0, random chunks ≈ 0.0.
- [ ] LSH index: inserts and queries work correctly.
- [ ] Pipeline integration: similarity stage runs between dedup and writer.
- [ ] CLI info displays similarity index statistics.
- [ ] Near-duplicate recall ≥ 90% at 70% similarity threshold.
- [ ] False positive rate < 5%.
- [ ] Memory budget enforcement works (LRU eviction).
- [ ] Modified Docker layer test shows extra savings beyond Phase 1.
- [ ] LEARNING.md updated with Phase 2 lessons.
