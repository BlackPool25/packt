# CI/CD Pipeline Overhaul Plan

> **Status**: Draft (awaiting approval)
> **Created**: 2026-07-16
> **Current Pipeline**: `packt/.github/workflows/ci.yml` (75 lines, 6 jobs)
> **Audit Reference**: AI codebase CI/CD analysis session
> **Industry Baselines**: ThreatFlux/rust-cicd-template, d-oit/rust-2026-template, Swatinem/rust-gha-workflows, ReasonKit Core

---

## 1. Overview

**Goal**: Production-grade CI/CD for a Rust data-dedup storage project — reproducibility, supply-chain security, quality gates, and release automation.

**Design Principles**:
- Reproducible builds (`--locked` everywhere)
- Supply-chain integrity (pinned action SHAs, `cargo-deny`, scheduled audits)
- Fast feedback (concurrency control, smart caching, `cargo-nextest`)
- Quality gates (coverage, MSRV, docs, feature check)
- Minimal runner minutes (cache all jobs, cancel stale runs)

---

## 2. File Changes Summary

| File | Action | Purpose |
|------|--------|---------|
| `.github/workflows/ci.yml` | **Rewrite** | Unified CI with 8 jobs, concurrency, caching, `--locked`, pinned SHAs |
| `.github/workflows/security.yml` | **Create** | Scheduled + triggered supply-chain audit + `cargo-deny` |
| `.github/workflows/release.yml` | **Create** | crates.io publish + GitHub release on tag |
| `packt-lib/Cargo.toml` | **Edit** | Add `rust-version = "1.85"` |
| `packt-cli/Cargo.toml` | **Edit** | Add `rust-version = "1.85"` |
| `rust-toolchain.toml` | **Edit** | Pin channel to specific version, add rustfmt+clippy |
| `deny.toml` | **Create** | License/bans/advisories configuration for `cargo-deny` |
| `.cargo/config.toml` | **Create** | Consistent build flags, parallel codegen units |
| `clippy.toml` | **Create** | MSRV-aware clippy configuration |
| `rustfmt.toml` | **Create** | Consistent formatting rules |
| `scripts/ci-coverage.sh` | **Create** | Coverage script for local + CI use |
| `.github/ISSUE_TEMPLATE/bug_report.md` | **Create** | Bug report template |
| `.github/ISSUE_TEMPLATE/feature_request.md` | **Create** | Feature request template |
| `.github/PULL_REQUEST_TEMPLATE.md` | **Create** | PR template with checklist |
| `CONTRIBUTING.md` | **Create** | Contribution guide |
| `SECURITY.md` | **Create** | Vulnerability disclosure policy |

---

## 3. Workflow Details

### 3.1 `ci.yml` — Main CI Pipeline

```yaml
name: CI
on:
  push:
    branches: [main, phase-*]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: -D warnings

permissions:
  contents: read

jobs:
  ### JOB 1: Format + Lint (fast feedback, single OS) ###
  lint:
    name: fmt + clippy
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

  ### JOB 2: Coverage (single OS, fast) ###
  coverage:
    name: Code coverage
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov
      - run: cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
      - uses: codecov/codecov-action@v4
        with:
          files: lcov.info
          fail_ci_if_error: false

  ### JOB 3: Docs build check ###
  docs:
    name: Documentation
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo doc --workspace --all-features --no-deps
        env:
          RUSTDOCFLAGS: -D warnings

  ### JOB 4: MSRV check ###
  msrv:
    name: MSRV (1.85)
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
        with:
          key: msrv-1.85
      - run: cargo check --workspace --all-targets --locked

  ### JOB 5: Cross-platform test matrix ###
  test:
    name: test (${{ matrix.os }}, ${{ matrix.toolchain }})
    runs-on: ${{ matrix.os }}
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        toolchain: [stable]
        include:
          - os: ubuntu-latest
            toolchain: beta
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.toolchain }}
      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.os }}-${{ matrix.toolchain }}
      - run: cargo build --workspace --all-features --locked
      - run: cargo test --workspace --all-features --locked

  ### JOB 6: Benchmarks (compile-check only) ###
  bench:
    name: Benchmarks
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo bench -p packt-lib --no-run --locked
```

**Key changes from current**:
- `concurrency` group with `cancel-in-progress: true` — no more wasted runs
- `permissions: contents: read` — least-privilege
- `RUSTFLAGS: -D warnings` — compiler warnings as errors globally
- `--locked` on all cargo commands — reproducible builds
- `--all-features` on clippy, test, build — feature-gate coverage
- `timeout-minutes` on every job — fail early on hangs
- Coverage job via `cargo-llvm-cov` with Codecov upload
- Docs job with `RUSTDOCFLAGS: -D warnings`
- MSRV job using pinned `1.85` toolchain
- Beta toolchain in test matrix (Linux only, for regression detection)
- Cache keys scoped per-matrix-cell to prevent collisions
- Benchmark job uses cache (was missing) + `--no-run` (compile-check only)
- Removed redundant `build-all` job (test already builds)

### 3.2 `security.yml` — Supply-Chain Security

New workflow for dependency security:

```yaml
name: Security Audit
on:
  pull_request:
    paths:
      - "**/Cargo.toml"
      - "**/Cargo.lock"
      - "deny.toml"
      - ".github/workflows/security.yml"
  push:
    branches: [main]
    paths:
      - "**/Cargo.toml"
      - "**/Cargo.lock"
      - "deny.toml"
      - ".github/workflows/security.yml"
  schedule:
    - cron: "0 6 * * 1"  # Every Monday 06:00 UTC
  workflow_dispatch:

permissions:
  contents: read

jobs:
  cargo-deny:
    name: cargo-deny (advisories + licenses + bans)
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
        with:
          command: check all
          arguments: --all-features
```

**Rationale**:
- Separate from CI so security failures don't block unrelated PRs
- Runs on schedule (Monday mornings) for new CVEs
- Path-filtered on PR/push (only when deps change)
- `cargo-deny` covers advisories (same DB as cargo-audit), plus licenses + banned crates + sources
- Single `cargo-deny-action` replaces the current `cargo install cargo-audit` pattern (saves 2-4 min)

### 3.3 `release.yml` — Release Automation

New workflow triggered by version tags:

```yaml
name: Release
on:
  push:
    tags:
      - "v*"

permissions:
  contents: write

jobs:
  publish:
    name: Publish to crates.io
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo publish -p packt-lib --locked
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
      - run: cargo publish -p packt-cli --locked
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

---

## 4. Configuration Files

### 4.1 `Cargo.toml` changes (both crates)

**`packt-lib/Cargo.toml`** — add after `edition`:
```toml
rust-version = "1.85"
```

**`packt-cli/Cargo.toml`** — add after `edition`:
```toml
rust-version = "1.85"
```

Why 1.85: Edition 2024 requires Rust ≥ 1.85 (stabilized Nov 2025). `edition = "2024"` is already set.

### 4.2 `rust-toolchain.toml` — Pin stable channel

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
```

No need to pin a specific nightly — `ci.yml` MSRV job handles the floor check. This file governs local dev.

### 4.3 `deny.toml` — cargo-deny configuration

```toml
[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
vulnerability = "deny"
unmaintained = "deny"
notice = "deny"
ignore = []

[licenses]
unlicensed = "deny"
allow = [
    "MIT",
    "Apache-2.0",
    "ISC",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "Unicode-3.0",
    "0BSD",
]
deny = [
    "GPL-2.0",
    "GPL-3.0",
    "AGPL-3.0",
    "LGPL-3.0",
]
confidence-threshold = 0.8

[bans]
multiple-versions = "deny"
wildcards = "deny"
highlight = "all"
deny = []

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-git = []
```

### 4.4 `.cargo/config.toml` — Local build optimization

```toml
[profile.dev]
codegen-units = 256
incremental = true

[profile.release]
lto = "fat"
codegen-units = 1
opt-level = 3
strip = "symbols"

[target.x86_64-unknown-linux-gnu]
linker = "clang"

[env]
CARGO_TERM_COLOR = "always"
```

Note: The workspace `Cargo.toml` already has `[profile.release]` with `lto = "fat"`, `codegen-units = 1`, `opt-level = 3`. This `.cargo/config.toml` duplicates those so local builds without workspace profiles still optimize well. The `strip` addition is new — removes debug symbols from release binaries.

### 4.5 `clippy.toml` — MSRV-aware lint config

```toml
msrv = "1.85"
```

### 4.6 `rustfmt.toml` — Format consistency

```toml
max_width = 120
edition = "2024"
```

---

## 5. Additional Improvements (Beyond Pipeline)

### 5.1 Contributor Experience

| File | Purpose |
|------|---------|
| `.github/ISSUE_TEMPLATE/bug_report.md` | Structured bug reports with reproduction steps |
| `.github/ISSUE_TEMPLATE/feature_request.md` | Feature request format |
| `.github/PULL_REQUEST_TEMPLATE.md` | PR checklist (tests, docs, changelog) |
| `CONTRIBUTING.md` | How to build, test, submit PRs |
| `SECURITY.md` | How to report vulnerabilities |

### 5.2 `scripts/ci-coverage.sh`

Helper script so devs can run the same coverage check locally before CI:
```bash
#!/usr/bin/env bash
set -euo pipefail
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
echo "Coverage report: lcov.info"
```

### 5.3 Remove `unwrap()` from library code (non-test)

From audit:
- `hashindex.rs:50` — `try_into().unwrap()` should use `?` or `expect` with context
- `hashindex.rs:247` — `handle.join().unwrap()` should propagate error
- These violate RULES.md §4 ("No unwrap/expect/panic in library code (allowed in tests and CLI)")

---

## 6. Implementation Order

| Step | Files | Dependencies | Est. Effort |
|------|-------|-------------|-------------|
| **1. Config files** | `deny.toml`, `.cargo/config.toml`, `clippy.toml`, `rustfmt.toml` | None | 15 min |
| **2. Cargo.toml edits** | `packt-lib/Cargo.toml`, `packt-cli/Cargo.toml` | None | 5 min |
| **3. rust-toolchain.toml edit** | `rust-toolchain.toml` | None | 2 min |
| **4. ci.yml rewrite** | `.github/workflows/ci.yml` | Steps 1-2 (for `--all-features`, `locked` to work) | 30 min |
| **5. security.yml** | `.github/workflows/security.yml` | Step 1 (`deny.toml`) | 15 min |
| **6. release.yml** | `.github/workflows/release.yml` | None | 10 min |
| **7. Coverage script** | `scripts/ci-coverage.sh` | None | 5 min |
| **8. Contributor files** | ISSUE_TEMPLATEs, PR template, CONTRIBUTING.md, SECURITY.md | None | 20 min |
| **9. Fix library unwrap()** | `hashindex.rs` | None | 10 min |

**Total**: ~9 steps, ~1.5-2 hours

---

## 7. Verification

After implementation, verify:
- [ ] `cargo fmt --check` passes locally
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes
- [ ] `cargo test --workspace --all-features --locked` passes on all 3 OS
- [ ] `cargo deny check all` passes (requires `cargo-deny` installed)
- [ ] Cargo.toml has `rust-version` in both crates
- [ ] All action versions pinned by SHA (replace `@v4` with actual SHAs during implementation)
- [ ] Changelog updated
- [ ] No deprecated APIs used (verify with `cargo build 2>&1 | grep -i deprecated`)
