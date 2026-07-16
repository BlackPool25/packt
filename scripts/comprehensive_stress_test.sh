#!/usr/bin/env bash
# Comprehensive stress test for packt Phase 1
# Tests: multiple image types, versions, cross-image dedup, edge cases, CLI commands
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
PASS="${GREEN}✓ PASS${NC}"; FAIL="${RED}✗ FAIL${NC}"; SKIP="${YELLOW}— SKIP${NC}"

COMPRESSOR_BIN="${CARGO_TARGET_DIR:-target}/release/packt-cli"
WORK_DIR=$(mktemp -d)
RESULTS_FILE="${WORK_DIR}/results.log"
TESTS_PASSED=0; TESTS_FAILED=0; TESTS_SKIPPED=0

echo -e "${CYAN}══════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}     Packt Phase 1 — Comprehensive Stress Test    ${NC}"
echo -e "${CYAN}══════════════════════════════════════════════════════${NC}"
echo "Started: $(date)"
echo "Work dir: $WORK_DIR"
echo ""

[ ! -f "$COMPRESSOR_BIN" ] && { echo -e "${YELLOW}Building release...${NC}" && cargo build --release -p packt-cli 2>&1 | tail -1; }
echo ""

# === TEST HELPER ===
run_test() {
    local name="$1"; shift
    echo -n "  ${name}... "
    if "$@" >> "$RESULTS_FILE" 2>&1; then
        echo -e "${PASS}"; TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${FAIL}"; TESTS_FAILED=$((TESTS_FAILED + 1))
        tail -3 "$RESULTS_FILE" | sed 's/^/      /'
    fi
}

# === TEST 1: Docker cross-version packt (same image, sequential versions) ===
echo -e "\n${CYAN}[Test Suite 1] Cross-Version Packt — Ubuntu LTS versions${NC}"

for IMG in "ubuntu:22.04" "ubuntu:23.04" "ubuntu:23.10" "ubuntu:24.04"; do
    VER="${IMG#ubuntu:}"
    run_test "Pull $IMG" docker pull "$IMG" --platform linux/amd64
done

# Test: packt across 4 Ubuntu versions
echo -e "\n${YELLOW}  1a. Ubuntu 22.04→24.04 packt (exact match across versions)${NC}"
U_STORE="${WORK_DIR}/u-store"
mkdir -p "$U_STORE"
extract_layers() {
    local img="$1" outdir="$2"
    local tag="${img#*:}"; tag="${tag//./}"
    docker save "$img" > "${WORK_DIR}/${tag}.tar"
    local edir="${WORK_DIR}/extract_${tag}"
    mkdir -p "$edir" "${outdir}"
    tar -xf "${WORK_DIR}/${tag}.tar" -C "$edir"
    if [ -f "$edir/manifest.json" ]; then
        for BLOB in "$edir"/blobs/sha256/*; do
            [ -f "$BLOB" ] || continue
            ft=$(file -b "$BLOB")
            if [[ "$ft" == *"tar archive"* ]] || [[ "$ft" == *"POSIX tar"* ]] || [[ "$ft" == *"gzip compressed"* ]] || [[ "$ft" == *"Zstandard"* ]] || [[ "$ft" == *"data"* ]]; then
                cp "$BLOB" "${outdir}/$(basename $BLOB).layer"
            fi
        done
    fi
    ls "${outdir}"/*.layer 2>/dev/null
}
for IMG in "ubuntu:22.04" "ubuntu:23.04" "ubuntu:23.10" "ubuntu:24.04"; do
    VER="${IMG#ubuntu:}"
    extract_layers "$IMG" "${WORK_DIR}/layers-${VER//./}"
    cat "${WORK_DIR}/layers-${VER//./}"/*.layer 2>/dev/null > "${WORK_DIR}/${VER//./}.bin" || true
done

run_test "Backup Ubuntu 22.04" $COMPRESSOR_BIN backup "${WORK_DIR}/2204.bin" "$U_STORE"
run_test "Backup Ubuntu 23.04" $COMPRESSOR_BIN backup "${WORK_DIR}/2304.bin" "$U_STORE"
run_test "Backup Ubuntu 23.10" $COMPRESSOR_BIN backup "${WORK_DIR}/2310.bin" "$U_STORE"
run_test "Backup Ubuntu 24.04" $COMPRESSOR_BIN backup "${WORK_DIR}/2404.bin" "$U_STORE"
run_test "Verify Ubuntu store" $COMPRESSOR_BIN verify "$U_STORE"
U_INFO=$($COMPRESSOR_BIN info "$U_STORE" 2>&1 | grep -oP 'Pack size:\s+\K[0-9]+')
U_ORIG=$(stat --format=%s "${WORK_DIR}/2204.bin" "${WORK_DIR}/2304.bin" "${WORK_DIR}/2310.bin" "${WORK_DIR}/2404.bin" 2>/dev/null | paste -sd+ | bc || echo 0)
echo -e "      Original: $(numfmt --to=iec-i --suffix=B $U_ORIG) → Stored: $(numfmt --to=iec-i --suffix=B $U_INFO) (Ratio: $(echo "scale=2; $U_ORIG / $U_INFO" | bc)x)"

# === TEST 2: Different image families (cross-image dedup) ===
echo -e "\n${CYAN}[Test Suite 2] Cross-Image Packt — Different image families${NC}"

for IMG in "alpine:3.18" "alpine:3.19" "alpine:3.20" "debian:11" "debian:12" "python:3.11-slim" "python:3.12-slim" "node:18-slim" "node:20-slim"; do
    echo -n "  Pulling $IMG... "
    if docker pull "$IMG" --platform linux/amd64 > /dev/null 2>&1; then echo "ok"; else echo "skip"; continue; fi
    SNAME=$(echo "$IMG" | tr '/:' '_')
    extract_layers "$IMG" "${WORK_DIR}/layers-${SNAME}"
    cat "${WORK_DIR}/layers-${SNAME}"/*.layer 2>/dev/null > "${WORK_DIR}/${SNAME}.bin" || true
done

# Backup all to one store — tests cross-image packt (common base layers)
echo -e "\n${YELLOW}  2a. All images → single store (cross-image dedup)${NC}"
CROSS_STORE="${WORK_DIR}/cross-store"
mkdir -p "$CROSS_STORE"
for IMG in "alpine:3.18" "alpine:3.19" "alpine:3.20" "debian:11" "debian:12" "python:3.11-slim" "python:3.12-slim" "node:18-slim" "node:20-slim"; do
    SNAME=$(echo "$IMG" | tr '/:' '_')
    BIN="${WORK_DIR}/${SNAME}.bin"
    [ -f "$BIN" ] || continue
    run_test "Backup $IMG" $COMPRESSOR_BIN backup "$BIN" "$CROSS_STORE"
done
run_test "Verify cross-store" $COMPRESSOR_BIN verify "$CROSS_STORE"
CROSS_INFO=$($COMPRESSOR_BIN info "$CROSS_STORE" 2>&1 | grep -oP 'Pack size:\s+\K[0-9]+')
CROSS_ORIG=0
for IMG in "alpine:3.18" "alpine:3.19" "alpine:3.20" "debian:11" "debian:12" "python:3.11-slim" "python:3.12-slim" "node:18-slim" "node:20-slim"; do
    SNAME=$(echo "$IMG" | tr '/:' '_')
    BIN="${WORK_DIR}/${SNAME}.bin"
    [ -f "$BIN" ] && CROSS_ORIG=$((CROSS_ORIG + $(stat --format=%s "$BIN")))
done
echo -e "      Original: $(numfmt --to=iec-i --suffix=B $CROSS_ORIG) → Stored: $(numfmt --to=iec-i --suffix=B $CROSS_INFO) (Ratio: $(echo "scale=2; $CROSS_ORIG / $CROSS_INFO" | bc)x)"

# === TEST 3: Edge cases ===
echo -e "\n${CYAN}[Test Suite 3] Edge Cases${NC}"

# 3a: Empty file
mkdir -p "${WORK_DIR}/edge-store"
touch "${WORK_DIR}/empty.bin"
run_test "3a. Empty file backup" $COMPRESSOR_BIN backup "${WORK_DIR}/empty.bin" "${WORK_DIR}/edge-store"

# 3b: Tiny file (1 byte)
echo -n "x" > "${WORK_DIR}/tiny.bin"
run_test "3b. Tiny file (1 byte)" $COMPRESSOR_BIN backup "${WORK_DIR}/tiny.bin" "${WORK_DIR}/edge-store"

# 3c: Sparse file (large, mostly zeros)
dd if=/dev/zero bs=1M count=100 2>/dev/null > "${WORK_DIR}/sparse.bin"
dd if=/dev/urandom bs=1K count=1 2>/dev/null >> "${WORK_DIR}/sparse.bin"
run_test "3c. Sparse file (100MB zeros + 1KB random)" $COMPRESSOR_BIN backup "${WORK_DIR}/sparse.bin" "${WORK_DIR}/edge-store"

# 3d: Already-compressed data (should still chunk but compression won't help much)
dd if=/dev/urandom bs=1M count=50 2>/dev/null > "${WORK_DIR}/random.bin"
run_test "3d. Random data (50MB, incompressible)" $COMPRESSOR_BIN backup "${WORK_DIR}/random.bin" "${WORK_DIR}/edge-store"

# 3e: Mixed content — binary + text
{
    dd if=/dev/urandom bs=1K count=512 2>/dev/null
    for i in $(seq 1 1000); do echo "log entry $i: this is repeated log data that should produce similar chunks"; done
    dd if=/dev/zero bs=1K count=512 2>/dev/null
} > "${WORK_DIR}/mixed.bin"
run_test "3e. Mixed binary+text+zeros" $COMPRESSOR_BIN backup "${WORK_DIR}/mixed.bin" "${WORK_DIR}/edge-store"

run_test "3f. Verify edge-case store" $COMPRESSOR_BIN verify "${WORK_DIR}/edge-store"

# === TEST 4: CLI edge cases ===
echo -e "\n${CYAN}[Test Suite 4] CLI Edge Cases${NC}"

run_test "4a. Missing source file" bash -c '"$0" backup /nonexistent/path /tmp/xyz 2>&1; [ $? -ne 0 ]' "$COMPRESSOR_BIN"
run_test "4b. Help output" bash -c '"$0" --help > /dev/null 2>&1' "$COMPRESSOR_BIN"
run_test "4c. Version output" bash -c '"$0" --version > /dev/null 2>&1' "$COMPRESSOR_BIN"
run_test "4d. Verify nonexistent store" bash -c '"$0" verify /nonexistent/path 2>&1; [ $? -ne 0 ]' "$COMPRESSOR_BIN"
run_test "4e. Info on empty store" bash -c 'mkdir -p /tmp/_empty_store && "$0" info /tmp/_empty_store > /dev/null 2>&1' "$COMPRESSOR_BIN"
run_test "4f. Backup with custom chunk size" $COMPRESSOR_BIN backup --chunk-size 16384 "${WORK_DIR}/mixed.bin" "${WORK_DIR}/edge-store"

# === TEST 5: Different chunk size performance ===
echo -e "\n${CYAN}[Test Suite 5] Chunk Size Comparison${NC}"
dd if=/dev/urandom bs=1M count=10 2>/dev/null > "${WORK_DIR}/perf.bin"
# Also add repetitive data (30% of file) to test dedup
dd if=/dev/zero bs=1K count=3000 2>/dev/null >> "${WORK_DIR}/perf.bin"

for CSIZE in 8192 16384 32768 65536; do
    S="${WORK_DIR}/cs-store-${CSIZE}"
    mkdir -p "$S"
    echo -n "  Avg chunk ${CSIZE}: "
    START=$(date +%s%N)
    $COMPRESSOR_BIN backup --chunk-size "$CSIZE" "${WORK_DIR}/perf.bin" "$S" 2>&1 | tail -1
    END=$(date +%s%N)
    STORE_SIZE=$(du -sb "$S" | cut -f1)
    ORIG_SIZE=$(stat --format=%s "${WORK_DIR}/perf.bin")
    RATIO=$(echo "scale=2; $ORIG_SIZE / $STORE_SIZE" | bc 2>/dev/null || echo "N/A")
    echo -e "      Stored: $(numfmt --to=iec-i --suffix=B $STORE_SIZE), Ratio: ${RATIO}x"
done

# === SUMMARY ===
echo ""
echo -e "${CYAN}══════════════════════════════════════════════════════${NC}"
echo -e "${CYAN}                    Results Summary                    ${NC}"
echo -e "${CYAN}══════════════════════════════════════════════════════${NC}"
echo -e "  Passed:  ${GREEN}${TESTS_PASSED}${NC}"
echo -e "  Failed:  ${RED}${TESTS_FAILED}${NC}"
echo -e "  Skipped: ${YELLOW}${TESTS_SKIPPED}${NC}"
echo ""

if [ "$TESTS_FAILED" -eq 0 ]; then
    echo -e "${GREEN}All tests passed. Phase 1 is solid.${NC}"
else
    echo -e "${RED}Some tests failed. Review above.${NC}"
fi

echo -e "\nUbuntu cross-version dedup:    ${U_ORIG} → ${U_INFO} ($(echo "scale=1; (1-$U_INFO.0/$U_ORIG)*100" | bc)% savings)"
if command -v restic &> /dev/null; then
    R_STORE="${WORK_DIR}/restic-compare"
    mkdir -p "$R_STORE"
    export RESTIC_REPOSITORY="$R_STORE" RESTIC_PASSWORD="test"
    restic init > /dev/null 2>&1 || true
    START=$(date +%s%N)
    for IMG in "alpine:3.18" "alpine:3.19" "alpine:3.20" "debian:11" "debian:12" "python:3.11-slim" "python:3.12-slim" "node:18-slim" "node:20-slim"; do
        SNAME=$(echo "$IMG" | tr '/:' '_')
        BIN="${WORK_DIR}/${SNAME}.bin"
        [ -f "$BIN" ] && restic backup "$BIN" > /dev/null 2>&1 || true
    done
    END=$(date +%s%N)
    R_SIZE=$(du -sb "$R_STORE" | cut -f1)
    R_RATIO=$(echo "scale=2; $CROSS_ORIG / $R_SIZE" | bc)
    echo "restic cross-image dedup:      ${CROSS_ORIG} → ${R_SIZE} (${R_RATIO}x)"
    echo "packt cross-image dedup:  ${CROSS_ORIG} → ${CROSS_INFO} ($(echo "scale=2; $CROSS_ORIG / $CROSS_INFO" | bc)x)"
fi

rm -rf "$WORK_DIR"
echo ""
echo -e "${GREEN}Done: $(date)${NC}"
