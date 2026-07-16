#!/usr/bin/env bash
set -euo pipefail
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
echo "Coverage report: lcov.info"
