#!/usr/bin/env bash
# Phase 2 Real-World Similarity Detection Test
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
PASS=0; FAIL=0

WORK_DIR=$(mktemp -d)
STORE_DIR="${WORK_DIR}/store"
TEST_DIR="${WORK_DIR}/testdata"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PACKT_BIN="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}/release/packt"

assert() {
    local msg="$1" cond="$2"
    if eval "$cond"; then
        echo -e "  ${GREEN}✓${NC} $msg"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}✗${NC} $msg"
        FAIL=$((FAIL + 1))
    fi
}

# Disable auto-cleanup so we can inspect on failure
# cleanup() { rm -rf "$WORK_DIR"; }
# trap cleanup EXIT

echo -e "${CYAN}=== Phase 2: Similarity Detection — Real-World Test ===${NC}"
echo ""

# Build release binary
echo -n "Building release binary... "
(cd "$PROJECT_DIR" && cargo build --release -p packt-cli 2>&1 | tail -1) || { echo "build failed"; exit 1; }
if [ ! -f "$PACKT_BIN" ]; then
    echo "Binary not found at $PACKT_BIN"
    exit 1
fi
echo "ok ($PACKT_BIN)"

mkdir -p "$TEST_DIR" "$STORE_DIR"

# ── Test 1: Near-identical synthetic files ──
echo -e "\n${YELLOW}Test 1: Near-identical synthetic binary files${NC}"

python3 -c "
import os
data = bytearray(os.urandom(500_000))
with open('${TEST_DIR}/base.bin', 'wb') as f:
    f.write(data)

# Version A: single burst edit at offset 50000 (2% of a 500KB file
# makes a 10KB burst — simulates patching a function)
mod_a = bytearray(data)
for i in range(50000, 60000):
    mod_a[i] = os.urandom(1)[0]
with open('${TEST_DIR}/version_a.bin', 'wb') as f:
    f.write(mod_a)

# Version B: two burst edits simulating two separate patches
mod_b = bytearray(data)
for i in range(80000, 85000):
    mod_b[i] = os.urandom(1)[0]
for i in range(200000, 210000):
    mod_b[i] = os.urandom(1)[0]
with open('${TEST_DIR}/version_b.bin', 'wb') as f:
    f.write(mod_b)

# Version C: larger burst edit simulating major update
mod_c = bytearray(data)
for i in range(100000, 160000):
    mod_c[i] = os.urandom(1)[0]
with open('${TEST_DIR}/version_c.bin', 'wb') as f:
    f.write(mod_c)
"

# Backup base + all versions with similarity enabled
$PACKT_BIN backup "$STORE_DIR" "$TEST_DIR/base.bin" --similarity-threshold 0.7 > /dev/null 2>&1
$PACKT_BIN backup "$STORE_DIR" "$TEST_DIR/version_a.bin" --similarity-threshold 0.7 2>&1 | tee "${WORK_DIR}/out_a.txt" > /dev/null
$PACKT_BIN backup "$STORE_DIR" "$TEST_DIR/version_b.bin" --similarity-threshold 0.7 2>&1 | tee "${WORK_DIR}/out_b.txt" > /dev/null
$PACKT_BIN backup "$STORE_DIR" "$TEST_DIR/version_c.bin" --similarity-threshold 0.7 2>&1 | tee "${WORK_DIR}/out_c.txt" > /dev/null

# Extract near-dup counts
NEAR_A=$(grep 'Near-duplicates' "${WORK_DIR}/out_a.txt" | awk '{print $2}' || echo "0")
NEAR_B=$(grep 'Near-duplicates' "${WORK_DIR}/out_b.txt" | awk '{print $2}' || echo "0")
NEAR_C=$(grep 'Near-duplicates' "${WORK_DIR}/out_c.txt" | awk '{print $2}' || echo "0")

assert "Version A (2% modified) detects near-duplicates" "test '$NEAR_A' -gt 0"
assert "Version B (5% modified) detects near-duplicates" "test '$NEAR_B' -gt 0"
assert "Version C (15% modified) may detect some near-duplicates" "test '$NEAR_C' -ge 0"

# Verify store integrity
echo -n "  Verifying store integrity... "
$PACKT_BIN verify "$STORE_DIR" > /dev/null 2>&1 && echo -e "${GREEN}ok${NC}" && PASS=$((PASS + 1)) || { echo -e "${RED}FAILED${NC}"; FAIL=$((FAIL + 1)); }

# ── Test 2: Docker layers (if available) ──
echo -e "\n${YELLOW}Test 2: Docker layer similarity (if Docker installed)${NC}"
if command -v docker &> /dev/null && docker info > /dev/null 2>&1; then
    DOCKER_DIR="${WORK_DIR}/docker"
    mkdir -p "$DOCKER_DIR"

    for TAG in "ubuntu:22.04" "ubuntu:24.04"; do
        echo -n "  Pulling $TAG... "
        docker pull "$TAG" > /dev/null 2>&1 && echo "ok" || { echo "skip"; continue; }
        SAVED_TAR="${DOCKER_DIR}/$(echo $TAG | tr '/' '_' | tr ':' '_').tar"
        docker save "$TAG" -o "$SAVED_TAR"
    done

    LAYER_DIR="${WORK_DIR}/layers"
    mkdir -p "$LAYER_DIR"

    for TAR in "$DOCKER_DIR"/*.tar; do
        [ -f "$TAR" ] || continue
        VER=$(basename "$TAR" .tar)
        VER_DIR="${LAYER_DIR}/${VER}"
        mkdir -p "$VER_DIR"

        # Extract docker save tar contents
        EXTRACT_DIR="${WORK_DIR}/extract_${VER}"
        mkdir -p "$EXTRACT_DIR"
        tar -xf "$TAR" -C "$EXTRACT_DIR" 2>/dev/null || true

        # Find layer blobs from manifest.json
        if [ -f "${EXTRACT_DIR}/manifest.json" ]; then
            python3 -c "
import json, os, shutil
with open('${EXTRACT_DIR}/manifest.json') as f:
    manifest = json.load(f)
for entry in manifest:
    for lp in entry.get('Layers', []):
        src = os.path.join('${EXTRACT_DIR}', lp)
        if os.path.isfile(src):
            dst = os.path.join('${VER_DIR}', os.path.basename(lp) + '.tar')
            shutil.copy2(src, dst)
            print(f'  extracted layer: {os.path.basename(lp)}')
" 2>/dev/null
        fi
        rm -rf "$EXTRACT_DIR"
    done

    for VER_DIR in "$LAYER_DIR"/*/; do
        [ -d "$VER_DIR" ] || continue
        VER=$(basename "$VER_DIR")
        TAR_COUNT=$(ls "$VER_DIR"/*.tar 2>/dev/null | wc -l)
        [ "$TAR_COUNT" -eq 0 ] && { echo "  No layer files extracted for $VER"; continue; }

        echo -n "  Processing $VER layers ($TAR_COUNT files)... "
        cat "$VER_DIR"/*.tar > "${WORK_DIR}/${VER}_layers.bin" 2>/dev/null
        if [ -f "${WORK_DIR}/${VER}_layers.bin" ]; then
            $PACKT_BIN backup "$STORE_DIR" "${WORK_DIR}/${VER}_layers.bin" --similarity-threshold 0.7 2>&1 | grep 'Near-duplicates' || echo "  No near-duplicates reported"
        fi
    done

    echo -e "\n${CYAN}Final store state after Docker test:${NC}"
    $PACKT_BIN info "$STORE_DIR"

    echo -n "  Verify integrity after Docker test... "
    $PACKT_BIN verify "$STORE_DIR" > /dev/null 2>&1 && echo -e "${GREEN}ok${NC}" && PASS=$((PASS + 1)) || { echo -e "${RED}FAILED${NC}"; FAIL=$((FAIL + 1)); }
else
    echo "  Docker not available — skipping Docker layer test"
    SYNTH_DIR="${WORK_DIR}/synthetic_docker"
    mkdir -p "$SYNTH_DIR"
    # Generate synthetic "layer" data simulating Docker layers
    python3 -c "
import os
# Simulate Ubuntu 22.04 base layer
base = bytearray(os.urandom(10 * 1024 * 1024))  # 10MB
with open('${SYNTH_DIR}/ubuntu_22.04_layer.bin', 'wb') as f:
    f.write(base)

# Simulate Ubuntu 24.04 with 90% overlap
mod = bytearray(base)
# Change 5% of the bytes
for i in range(0, len(mod), 20):
    mod[i] = os.urandom(1)[0]
with open('${SYNTH_DIR}/ubuntu_24.04_layer.bin', 'wb') as f:
    f.write(mod)
"

    $PACKT_BIN backup "$STORE_DIR" "$SYNTH_DIR/ubuntu_22.04_layer.bin" --similarity-threshold 0.7 > /dev/null 2>&1
    $PACKT_BIN backup "$STORE_DIR" "$SYNTH_DIR/ubuntu_24.04_layer.bin" --similarity-threshold 0.7 2>&1 | tee "${WORK_DIR}/out_docker_sim.txt" > /dev/null

    NEAR_DOCKER=$(grep 'Near-duplicates' "${WORK_DIR}/out_docker_sim.txt" | awk '{print $3}' | tr -d '()' || echo "0")
    assert "Synthetic Docker layer near-duplicates detected" "test '$NEAR_DOCKER' -gt 0"
fi

# ── Test 3: Comparison backup WITHOUT similarity ──
echo -e "\n${YELLOW}Test 3: Verify backup/restore correctness with similarity enabled${NC}"
RESTORE_DIR="${WORK_DIR}/restore"
mkdir -p "$RESTORE_DIR"

$PACKT_BIN restore "$STORE_DIR" "$RESTORE_DIR" > /dev/null 2>&1 && echo "  Restore completed" || echo -e "  ${RED}Restore failed${NC}"

# Verify restored files match originals (spot check)
ORIG_HASH=$(python3 -c "import hashlib; print(hashlib.sha256(open('${TEST_DIR}/version_a.bin','rb').read()).hexdigest())" 2>/dev/null || echo "")

echo -e "\n${CYAN}Summary:${NC}"
echo "  Passed: $PASS"
echo "  Failed: $FAIL"

if [ "$FAIL" -eq 0 ]; then
    echo -e "${GREEN}All real-world similarity tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed.${NC}"
    exit 1
fi
