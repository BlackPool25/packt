#!/usr/bin/env bash
# packt vs restic: comprehensive comparison
# Tests: speed, dedup ratio, similarity detection value
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PACKT_BIN="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}/release/packt"
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
PASS=0; FAIL=0

assert() { local msg="$1" cond="$2"
    if eval "$cond"; then echo -e "  ${GREEN}✓${NC} $msg"; PASS=$((PASS + 1))
    else echo -e "  ${RED}✗${NC} $msg"; FAIL=$((FAIL + 1)); fi
}

cleanup_all() { rm -rf /tmp/packt_bench_*; }

echo -e "${CYAN}=== packt vs restic: Comprehensive Comparison ===${NC}"
echo ""

echo -n "Building packt release... "
(cd "$PROJECT_DIR" && cargo build --release -p packt-cli -q 2>&1) && echo "ok" || { echo "failed"; exit 1; }
[ -f "$PACKT_BIN" ] || { echo "Binary not found at $PACKT_BIN"; exit 1; }

# ═══════════════════════════════════════════════════════════════
# TEST 1: Speed — 200MB random data (incompressible)
# ═══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}Test 1: Speed — 200MB Random Data${NC}"
W1=$(mktemp -d)
dd if=/dev/urandom of="$W1/random.bin" bs=1M count=200 2>/dev/null
echo "  Source: 200 MiB (no duplicates, incompressible)"

rm -rf "$W1/store"
START=$(date +%s%N); $PACKT_BIN backup "$W1/store" "$W1/random.bin" --similarity-threshold 0 > /dev/null 2>&1; END=$(date +%s%N)
PACKT_NOSIM_MS=$(( (END - START) / 1000000 ))

rm -rf "$W1/store"
START=$(date +%s%N); $PACKT_BIN backup "$W1/store" "$W1/random.bin" --similarity-threshold 0.7 > /dev/null 2>&1; END=$(date +%s%N)
PACKT_SIM_MS=$(( (END - START) / 1000000 ))

rm -rf "$W1/store"
restic init --repo "$W1/store" --insecure-no-password > /dev/null 2>&1
START=$(date +%s%N); restic backup --repo "$W1/store" --insecure-no-password "$W1/random.bin" > /dev/null 2>&1; END=$(date +%s%N)
RESTIC_MS=$(( (END - START) / 1000000 ))

echo "  packt (no sim):  ${PACKT_NOSIM_MS}ms"
echo "  packt (sim):     ${PACKT_SIM_MS}ms"
echo "  restic:          ${RESTIC_MS}ms"
assert "packt no-sim faster than restic" "test $PACKT_NOSIM_MS -lt $RESTIC_MS"
echo "  speedup vs restic: $(echo "scale=1; $RESTIC_MS / $PACKT_NOSIM_MS" | bc)x"

# ═══════════════════════════════════════════════════════════════
# TEST 2: Dedup — Ubuntu Docker layers (as many versions as available)
# ═══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}Test 2: Dedup Ratio — Ubuntu Docker Layers${NC}"

UBUNTU_VERSIONS=()
for VER in "22.04" "23.04" "23.10" "24.04"; do
    if docker pull "ubuntu:${VER}" -q > /dev/null 2>&1; then
        UBUNTU_VERSIONS+=("$VER")
        echo "  ubuntu:${VER} available"
    else
        echo "  ubuntu:${VER} EOL — skipping"
    fi
done

W2=$(mktemp -d)
DATA_DIR="$W2/data"
LAYER_DIR="$W2/layers"
mkdir -p "$DATA_DIR" "$LAYER_DIR"

for VER in "${UBUNTU_VERSIONS[@]}"; do
    echo -n "  Extracting ubuntu:${VER}... "
    mkdir -p "${LAYER_DIR}/${VER}" "$W2/extract_${VER}"
    docker save "ubuntu:${VER}" > "$W2/${VER}.tar" 2>/dev/null
    tar -xf "$W2/${VER}.tar" -C "$W2/extract_${VER}" 2>/dev/null || true
    if [ -f "$W2/extract_${VER}/manifest.json" ]; then
        python3 -c "
import json, os, shutil
with open('$W2/extract_${VER}/manifest.json') as f: manifest = json.load(f)
for entry in manifest:
    for lp in entry.get('Layers', []):
        src = os.path.join('$W2/extract_${VER}', lp)
        if os.path.isfile(src):
            shutil.copy2(src, '$LAYER_DIR/${VER}/' + os.path.basename(lp) + '.tar')
" 2>/dev/null
    fi
    cat "$LAYER_DIR/${VER}"/*.tar > "${DATA_DIR}/ubuntu_${VER}.bin" 2>/dev/null
    SIZE=$(stat -c%s "${DATA_DIR}/ubuntu_${VER}.bin" 2>/dev/null || echo 0)
    echo "$(numfmt --to=iec-i --suffix=B $SIZE)"
    rm -rf "$W2/extract_${VER}"
done
rm -rf "$LAYER_DIR"

SOURCE_FILES=("$DATA_DIR"/*.bin)
SOURCE_SIZE=$(du -sb "$DATA_DIR" | cut -f1)
N_VERSIONS=${#SOURCE_FILES[@]}
echo "  Total source: $(numfmt --to=iec-i --suffix=B $SOURCE_SIZE) across $N_VERSIONS versions"

# packt WITH similarity
rm -rf "$W2/store_sim"
SIM_START=$(date +%s%N)
for F in "${SOURCE_FILES[@]}"; do $PACKT_BIN backup "$W2/store_sim" "$F" --similarity-threshold 0.7 > /dev/null 2>&1; done
$PACKT_BIN verify "$W2/store_sim" > /dev/null 2>&1
SIM_END=$(date +%s%N)
SIM_MS=$(( (SIM_END - SIM_START) / 1000000 ))
SIM_SIZE=$(du -sb "$W2/store_sim" | cut -f1)
SIM_RATIO=$(echo "scale=2; $SOURCE_SIZE / $SIM_SIZE" | bc)

# packt WITHOUT similarity
rm -rf "$W2/store_nosim"
NOSIM_START=$(date +%s%N)
for F in "${SOURCE_FILES[@]}"; do $PACKT_BIN backup "$W2/store_nosim" "$F" --similarity-threshold 0 > /dev/null 2>&1; done
$PACKT_BIN verify "$W2/store_nosim" > /dev/null 2>&1
NOSIM_END=$(date +%s%N)
NOSIM_MS=$(( (NOSIM_END - NOSIM_START) / 1000000 ))
NOSIM_SIZE=$(du -sb "$W2/store_nosim" | cut -f1)
NOSIM_RATIO=$(echo "scale=2; $SOURCE_SIZE / $NOSIM_SIZE" | bc)

# restic
rm -rf "$W2/store_restic"
restic init --repo "$W2/store_restic" --insecure-no-password > /dev/null 2>&1
RESTIC_START=$(date +%s%N)
for F in "${SOURCE_FILES[@]}"; do
    restic backup --repo "$W2/store_restic" --insecure-no-password "$F" > /dev/null 2>&1
done
RESTIC_END=$(date +%s%N)
RESTIC_MS=$(( (RESTIC_END - RESTIC_START) / 1000000 ))
RESTIC_SIZE=$(du -sb "$W2/store_restic" | cut -f1)
RESTIC_RATIO=$(echo "scale=2; $SOURCE_SIZE / $RESTIC_SIZE" | bc)

echo ""
echo "  packt (sim):     $(numfmt --to=iec-i --suffix=B $SIM_SIZE) stored, ${SIM_RATIO}x ratio, ${SIM_MS}ms"
echo "  packt (no sim):  $(numfmt --to=iec-i --suffix=B $NOSIM_SIZE) stored, ${NOSIM_RATIO}x ratio, ${NOSIM_MS}ms"
echo "  restic:          $(numfmt --to=iec-i --suffix=B $RESTIC_SIZE) stored, ${RESTIC_RATIO}x ratio, ${RESTIC_MS}ms"

if [ "$N_VERSIONS" -ge 3 ]; then
    assert "packt no-sim ratio >= restic ratio" "echo \"$NOSIM_RATIO >= $RESTIC_RATIO\" | bc -l | grep -q 1"
    echo "  packt advantage vs restic: $(echo "scale=2; $NOSIM_RATIO / $RESTIC_RATIO" | bc)x better ratio"
fi

# ═══════════════════════════════════════════════════════════════
# TEST 3: Similarity value — modified binary file
# ═══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}Test 3: Similarity Detection — Modified Binary File${NC}"
W3=$(mktemp -d)

python3 -c "
import os
base = bytearray(i % 251 for i in range(50 * 1024 * 1024))
with open('$W3/base.bin', 'wb') as f: f.write(base)
mod = bytearray(base)
for i in range(100000, len(mod), 200):
    mod[i] = 255
with open('$W3/modified.bin', 'wb') as f: f.write(mod)
"
echo "  Base: 50MB structured, Modified: 50MB (0.5% bytes changed, CDC-aligned)"

# packt WITH similarity: backup base, then modified
rm -rf "$W3/store_sim"
$PACKT_BIN backup "$W3/store_sim" "$W3/base.bin" --similarity-threshold 0.7 > /dev/null 2>&1
SIM_OUT=$($PACKT_BIN backup "$W3/store_sim" "$W3/modified.bin" --similarity-threshold 0.7 2>&1)
SIM_SIZE=$(du -sb "$W3/store_sim" | cut -f1)
SIM_NEAR=$(echo "$SIM_OUT" | grep 'Near-duplicates' | awk '{print $2}' || echo "0")
SIM_NEAR_PCT=$(echo "$SIM_OUT" | grep 'Near-duplicates' | awk '{print $3}' | tr -d '()%' || echo "0")
echo "  With similarity:  $(numfmt --to=iec-i --suffix=B $SIM_SIZE) stored, $SIM_NEAR near-duplicates (${SIM_NEAR_PCT}%)"

# packt WITHOUT similarity: backup base, then modified
rm -rf "$W3/store_nosim"
$PACKT_BIN backup "$W3/store_nosim" "$W3/base.bin" --similarity-threshold 0 > /dev/null 2>&1
NOSIM_OUT=$($PACKT_BIN backup "$W3/store_nosim" "$W3/modified.bin" --similarity-threshold 0 2>&1)
NOSIM_SIZE=$(du -sb "$W3/store_nosim" | cut -f1)
echo "  Without sim:      $(numfmt --to=iec-i --suffix=B $NOSIM_SIZE) stored"

# restic: backup both files
rm -rf "$W3/store_restic"
restic init --repo "$W3/store_restic" --insecure-no-password > /dev/null 2>&1
restic backup --repo "$W3/store_restic" --insecure-no-password "$W3/base.bin" > /dev/null 2>&1
restic backup --repo "$W3/store_restic" --insecure-no-password "$W3/modified.bin" > /dev/null 2>&1
RESTIC3_SIZE=$(du -sb "$W3/store_restic" | cut -f1)
echo "  restic:           $(numfmt --to=iec-i --suffix=B $RESTIC3_SIZE) stored"

echo ""
echo "  Similarity saves: $(numfmt --to=iec-i --suffix=B $((NOSIM_SIZE - SIM_SIZE))) compared to no-sim"

assert "similarity detects near-duplicates on modified file" "test $SIM_NEAR -gt 0"
assert "similarity reduces stored size vs no-sim" "test $SIM_SIZE -lt $NOSIM_SIZE"
assert "packt with sim stores less than restic" "test $SIM_SIZE -lt $RESTIC3_SIZE"

# ═══════════════════════════════════════════════════════════════
# CLEANUP
# ═══════════════════════════════════════════════════════════════
echo -e "\n${CYAN}Final Summary:${NC}"
echo "  Test 1 (Speed):           $( [ $PACKT_NOSIM_MS -lt $RESTIC_MS ] && echo 'packt wins' || echo 'restic wins' )"
echo "  Test 2 (Dedup ratio):     $( [ "$N_VERSIONS" -ge 3 ] && ( echo "$NOSIM_RATIO >= $RESTIC_RATIO" | bc -l | grep -q 1 && echo 'packt wins' || echo 'restic wins' ) || echo 'N/A (<3 versions)' )"
echo "  Test 3 (Similarity):      $( [ "$SIM_NEAR" -gt 0 ] && echo 'similarity active' || echo 'no near-duplicates' )"
echo ""
echo -e "${GREEN}Passed: $PASS  Failed: $FAIL${NC}"
[ "$FAIL" -eq 0 ] || echo -e "${RED}Some tests failed${NC}"

rm -rf "$W1" "$W2" "$W3"
