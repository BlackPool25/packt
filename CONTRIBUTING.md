# Contributing to Packt

Thank you for your interest in contributing to Packt! This document provides
guidelines and instructions for contributing.

## Getting Started

1.  Ensure the Rust toolchain is installed:
    ```bash
    rustup install stable
    rustup default stable
    ```

2.  Clone the repository:
    ```bash
    git clone https://github.com/BlackPool25/packt.git
    cd packt/compressor
    ```

3.  Build and test:
    ```bash
    cargo build --workspace
    cargo test --workspace --all-targets
    ```

## Development Workflow

1.  Create a feature branch from `main`:
    ```bash
    git checkout -b feat/my-feature
    ```

2.  Make changes following the coding standards below.

3.  Run quality checks before committing:
    ```bash
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --all-targets
    ```

4.  Update `CHANGELOG.md` with your changes following the
    [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.

5.  Commit with a signed-off-by message:
    ```bash
    git commit -s -m "feat: my feature description"
    ```

6.  Push and open a Pull Request:
    ```bash
    git push origin feat/my-feature
    ```

## Coding Standards

*   **Safety**: `#![forbid(unsafe_code)]` — no unsafe in library code
*   **Errors**: No `unwrap()` or `expect()` in library code (allowed in CLI
    and tests only)
*   **Error types**: Use `thiserror` for all error types
*   **Documentation**: All public API items must have doc comments with
    examples
*   **Linting**: Follow Clippy pedantic lints (`cargo clippy -D warnings`)
*   **Formatting**: Run `cargo fmt --all` before every commit

## Commit Convention

This project uses [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>: <description>

[optional body]

Signed-off-by: Your Name <your.email@example.com>
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`.

## Pull Request Process

1.  Ensure all CI checks pass (fmt, clippy, test, build)
2.  Update `CHANGELOG.md` with your changes
3.  Request review from maintainers
4.  Squash-merge when approved

## Phase Discipline

*   Each phase builds on the previous without modifying stable modules
*   Read `DECISIONS.md` before making architecture decisions
*   Read `LEARNING.md` to avoid repeating past mistakes

## CI/CD

*   All PRs run: fmt, clippy, tests (3 OS), coverage, MSRV check, docs build
*   Supply-chain: `cargo-deny` runs on dependency changes
*   All builds use `--locked` for reproducibility
*   Never push directly to `main` — all changes go through PRs
