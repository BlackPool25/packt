# Agent Rules & Self-Learning

> **Purpose**: Rules for AI agents building this project. Read before any implementation work.
> **Last Updated**: 2026-07-16
> **Current Phase**: Phase 2 (Similarity Detection)
> **Project Name**: packt
> **Repository**: https://github.com/BlackPool25/packt

---

## 1. Golden Rules

### 1.1 Correctness Over Everything
- Storage software that loses data is abandoned forever.
- Every write path must have integrity verification (BLAKE3 checksums).
- Never skip error handling on IO operations.
- Never suppress compiler warnings or type errors.
- Property-based tests (proptest) required for: chunking determinism, compression round-trip, chunk coverage.

### 1.2 Phase Discipline
- **Phase 2 focus**: Similarity detection only. No delta compression, no encryption, no cloud storage.
- Do NOT modify Phase 1 modules (chunking, hashing, pack format, store, index) unless fixing a bug.
- Each phase must be independently useful.

### 1.3 Research Before Coding
- Read DECISIONS.md before making any new decision.
- Read LEARNING.md to avoid repeating mistakes.
- Read PROJECT_RESEARCH_COMPLETE.md Section 3 (Similarity Detection) before implementing Phase 2.
- Read the Phase 2 handoff document at `.omo/plans/phase-2-handoff.md`.
- All phases, rules already read for this project.

### 1.4 Evidence Requirements
Tasks are NOT complete without:
- `lsp_diagnostics` clean on all changed files
- Relevant tests pass (new + existing + `cargo test --workspace --all-targets`)
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo fmt --check` clean
- Entry added to LEARNING.md documenting what was learned
- No unwrap() in production code (allowed in tests and CLI only)

### 1.5 Git Workflow — PRs Only
- **NEVER push commits directly to `main`.** Never.
- All changes MUST go through GitHub Pull Requests:
  1. Create a feature/fix branch from `main`
  2. Commit incrementally with meaningful messages
  3. Push branch and open a PR to `main`
  4. Wait for CI to pass on the PR
  5. Only merge when CI is green
- If CI fails on the PR, fix the issue on the branch before merging.
- Squash-merge PRs to keep history clean.
- Delete the branch after merging.

---

## 2. Phase 2-Specific Rules

### 2.1 Similarity Detection Architecture
- Implement as a separate module `packt-lib/src/similarity/` — do NOT modify existing Phase 1 modules.
- The similarity index is an ADDITIVE component: chunks flow through it only if they fail exact dedup.
- Trait design: `SimilarityDetector` with method `fn find_similar(&self, hash: &Hash, data: &[u8]) -> Option<Hash>`

### 2.2 Byte Shingle Tokenizer (2a)
- Must work directly on raw `&[u8]` — no text preprocessing.
- Configurable shingle size: 4-byte (default), 8-byte, 16-byte, 32-byte.
- Must handle chunks smaller than shingle size gracefully (pad or skip).
- Research reference: PROJECT_RESEARCH_COMPLETE.md Section 3.6 (Shingling Strategies)

### 2.3 MinHash Signature (2b)
- Use `txtfp` crate's MinHash internals adapted for byte shingles (NOT text tokens).
- Must produce deterministic signatures (same chunk → same signature across runs and platforms).
- Default: k=128 hash functions for the MinHash signature.
- Performance target: < 200µs per 32KB chunk.

### 2.4 LSH Index (2c)
- Banded LSH with configurable b (bands) and r (rows per band).
- Default: b=20, r=6 (threshold ≈ 0.74, good recall for highly similar chunks).
- Must support LRU eviction with configurable memory budget (default: 1GB).
- Query must return candidates sorted by similarity score.

### 2.5 Integration with Existing Pipeline (2d)
- Similarity check runs AFTER exact dedup, BEFORE writing to store.
- Integration point: add a stage in the pipeline between DedupStage and WriterStage.
- When similarity detects a match: store as-is for now (delta compression comes in Phase 3).

### 2.6 Testing Requirements (2f)
- Near-duplicate detection accuracy: ≥ 90% recall at 70% similarity threshold.
- False positive rate: < 5%.
- Memory budget enforcement: verify LRU eviction triggers at configured limit.
- Determinism: same chunk → same MinHash signature across runs.

### 2.7 Phase 2 Commandments
- Do NOT add stubs for Phase 3 (delta compression).
- Do NOT modify the pack format (that's Phase 3's job).
- Do NOT modify the ContentStore or DedupIndex traits.
- The similarity index is in-memory only (tiered storage is Phase 5).

---

## 3. Phase 1 Code Architecture (Reference)

### 3.1 File Organization
```
packt/
├── packt-lib/src/
│   ├── lib.rs              # Re-exports: chunking, hash, store, index, pipeline, types
│   ├── error.rs            # PacktError enum (thiserror)
│   ├── types.rs            # Hash, Chunk, ChunkConfig, PackLocation
│   ├── chunking/           # Chunker trait + FastCDC v2020 impl
│   ├── hash/               # ContentHasher trait + Blake3Hasher impl
│   ├── store/              # ContentStore trait + LocalStore + pack format
│   ├── index/              # DedupIndex trait + HashIndex (DashMap + Bloom filter)
│   ├── pipeline/           # BackupPipeline, stage modules, BackupStats
│   └── (util/ removed — BufferPool was dead code)
├── packt-cli/src/
│   ├── main.rs             # clap setup: backup, restore, info, verify, benchmark
│   ├── backup.rs           # backup + manifest saving with metadata
│   ├── restore.rs          # manifest-based restore with per-chunk verification
│   ├── info.rs             # store statistics
│   └── verify.rs           # full integrity verification
├── packt-lib/tests/        # Integration tests
└── packt-lib/benches/      # Criterion benchmarks
```

### 3.2 Key Traits (Do NOT Modify)
```rust
pub trait Chunker: Send + Sync { fn chunk(&self, data: &[u8]) -> Vec<Chunk>; ... }
pub trait ContentHasher: Send + Sync { fn hash(&self, data: &[u8]) -> Hash; ... }
pub trait ContentStore: Send + Sync { fn put(&self, hash: &Hash, data: &[u8]) -> Result<PackLocation>; ... }
pub trait DedupIndex: Send + Sync { fn insert(&self, hash: Hash, location: PackLocation); ... }
```

### 3.3 Pipeline (Where to Integrate Phase 2)
The pipeline uses **streaming chunking** (fastcdc::StreamCDC) — no full file in memory.
Flow: StreamCDC → HasherStage → DedupStage → WriterStage (channel-based, backpressure).
Phase 2 adds: ... → DedupStage → **SimilarityStage** → WriterStage.
The `DedupMessage` enum may need extension to carry similarity info.

---

## 4. Coding Standards

- `#![forbid(unsafe_code)]` in lib.rs.
- Clippy pedantic: `#![warn(clippy::pedantic)]`.
- No unwrap/expect/panic in library code (allowed in CLI and tests).
- Use thiserror for error types.
- All public API items must have doc comments with examples.
- Run `cargo fmt --all` before every commit.
- Run `cargo clippy --workspace --all-targets -- -D warnings` before every commit.
- Run `cargo test --workspace --all-targets` before every commit.

---

## 5. Testing Requirements

- Unit tests for every public function.
- Property tests (proptest) for: determinism, round-trip integrity.
- Integration tests for end-to-end backup/restore on real files.
- Benchmarks (criterion) for every pipeline stage.
- Test files: use tempfile crate for temp directories.

---

## 6. Self-Learning

Add entries to LEARNING.md when:
1. Bug found in production code — document root cause and fix.
2. Performance regression detected — document the change that caused it.
3. API design mistake — something had to be refactored because the initial design was wrong.
4. Research finding — new information that changes a design decision.

Before starting any task, read LEARNING.md from top.

---

## 7. Dependency Policy (Phase 2 Additions)

### Approved New Dependencies for Phase 2
| Crate | Purpose | Justification |
|-------|---------|---------------|
| `txtfp` | MinHash primitive internals | Only for hash function adaptation — DO NOT use text tokenizer |

### Phase 1 Dependencies (Active)
fastcdc, blake3, zstd-rs, serde, postcard, rayon, crossbeam-channel, dashmap, tracing, thiserror, hex, cfg-if

### Phase 1 Dev-Dependencies (Active)
tempfile, criterion, proptest, rand, walkdir

### Removed Dependencies (Phase 1 hardening)
- `memmap2` (was declared but never used)
- `bincode` (replaced by postcard — original bincode v2 project is defunct as of 2025)
