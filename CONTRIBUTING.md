# Contributing to packt

## Getting Started
1. Ensure Rust toolchain: `rustup install stable && rustup default stable`
2. Clone: `git clone https://github.com/BlackPool25/packt`
3. Build: `cargo build --workspace`
4. Test: `cargo test --workspace`

## Development Workflow
1. Create a feature branch from `main`
2. Make changes following coding standards below
3. Run quality checks before committing:
   ```
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
   cargo test --workspace --all-features --locked
   ```
4. Update CHANGELOG.md with your changes
5. Submit a PR

## Coding Standards
- `#![forbid(unsafe_code)]` — no unsafe in library
- No `unwrap()` in library code (allowed in CLI and tests)
- Use `thiserror` for error types
- All public API items must have doc comments
- Follow Clippy pedantic lints

## Phase Discipline
- Each phase builds on the previous without modifying it
- Read DECISIONS.md before making architecture decisions
- Read RULES.md before starting implementation

## CI/CD
- All PRs run: fmt, clippy, tests (3 OS), coverage, MSRV check, docs build
- Supply-chain: cargo-deny runs on dependency changes
- All builds use `--locked` for reproducibility
