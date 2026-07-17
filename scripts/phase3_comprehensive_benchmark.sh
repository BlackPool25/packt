#!/usr/bin/env bash
# Phase 3 Comprehensive Real-World Benchmark
# Tests: Docker cross-image, daily backups, VM snapshots, mixed workloads
set -euo pipefail

PACKT="$(cd "$(dirname "$0")/.." && pwd)/target/release/packt"
WORK_DIR=$(mktemp -d)
export WORK_DIR

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
pass() { echo -e "  ${GREEN}✓${NC} $1"; }
fail() { echo -e "  ${RED}✗${NC} $1"; }

echo -e "${CYAN}====================================================${NC}"
echo -e "${CYAN}  PHASE 3 COMPREHENSIVE REAL-WORLD BENCHMARK${NC}"
echo -e "${CYAN}  Date: $(date)${NC}"
echo -e "${CYAN}====================================================${NC}"

cd "$(dirname "$0")/.."

# Build release
echo -n "Building release... "
cargo build --release -p packt-cli 2>&1 | tail -1
echo "done"

# ── Generate datasets ──
echo -e "\n${YELLOW}[1/4] Generating datasets${NC}"

# Docker cross-image
echo -n "  Docker layers (5 images)... "
for IMG in "ubuntu:22.04" "ubuntu:24.04" "alpine:3.20" "debian:12" "python:3.12-slim"; do
    docker pull "$IMG" --platform linux/amd64 > /dev/null 2>&1
    SNAME=$(echo "$IMG" | tr '/:' '_')
    EX_DIR="${WORK_DIR}/extract_${SNAME}"
    mkdir -p "$EX_DIR"
    docker save "$IMG" 2>/dev/null | tar -xf - -C "$EX_DIR" 2>/dev/null
    # Find layer blobs from manifest.json
    > "${WORK_DIR}/${SNAME}.layers"
    python3 -c "
import json, os
ex = '${EX_DIR}'
with open(os.path.join(ex, 'manifest.json')) as f:
    manifest = json.load(f)
for entry in manifest:
    for lp in entry.get('Layers', []):
        src = os.path.join(ex, lp)
        if os.path.isfile(src):
            with open(src, 'rb') as fin:
                with open(os.path.join('${WORK_DIR}', '${SNAME}.layers'), 'ab') as fout:
                    fout.write(fin.read())
" 2>/dev/null || true
    rm -rf "$EX_DIR"
done
DOCKER_FILES=$(ls "${WORK_DIR}"/*.layers 2>/dev/null || true)
echo "done"

# Daily backups (proper Python, no shell var issues)
echo -n "  10 daily backups (100MB each, 5% churn)... "
python3 << 'PYEOF'
import os, random
random.seed(42)
work_dir = os.environ.get('WORK_DIR', '/tmp')
base = bytearray(os.urandom(100 * 1024 * 1024))
for day in range(10):
    v = bytearray(base)
    indices = random.sample(range(len(v)), int(len(v) * 0.05))
    for pos in indices:
        v[pos] = random.getrandbits(8)
    with open(os.path.join(work_dir, f'day{day}.bin'), 'wb') as f:
        f.write(v)
PYEOF
DAILY_FILES=$(ls "${WORK_DIR}"/day[0-9].bin 2>/dev/null || true)
echo "done ($(echo $DAILY_FILES | wc -w) files)"

# VM snapshots
echo -n "  4 VM snapshots (50MB each, 2% page churn)... "
python3 << 'PYEOF'
import os, random
random.seed(7)
work_dir = os.environ.get('WORK_DIR', '/tmp')
base = bytearray()
for _ in range(200):
    t = random.random()
    if t < 0.3: base += b'\x00' * 262144
    elif t < 0.6: base += bytes(random.getrandbits(8) for _ in range(262144))
    else:
        p = bytes(random.getrandbits(8) for _ in range(256))
        base += p * 1024
with open(os.path.join(work_dir, 'vm_v1.bin'), 'wb') as f: f.write(base)
for ver in range(2, 5):
    v = bytearray(base)
    for _ in range(int(len(v) * 0.02 / 4096)):
        pos = random.randint(0, len(v) - 4096)
        v[pos:pos+4096] = bytes(random.getrandbits(8) for _ in range(4096))
    with open(os.path.join(work_dir, f'vm_v{ver}.bin'), 'wb') as f: f.write(v)
PYEOF
VM_FILES=$(ls "${WORK_DIR}"/vm_v*.bin 2>/dev/null || true)
echo "done ($(echo $VM_FILES | wc -w) files)"

ALL_FILES=$(find "$WORK_DIR" -maxdepth 1 \( -name '*.layers' -o -name 'day*.bin' -o -name 'vm_v*.bin' \) -type f | sort)
ORIG_TOTAL=$(du -sb $ALL_FILES 2>/dev/null | awk '{total+=$1} END{print total}')
echo "  Total dataset: $(numfmt --to=iec-i --suffix=B $ORIG_TOTAL) ($(echo $ALL_FILES | wc -w) files)"

# ── Backup benchmarks ──
echo -e "\n${YELLOW}[2/4] Running backups${NC}"

run_packt() {
    local store="$1"; shift
    mkdir -p "$store"
    local start=$(date +%s%N)
    for f in "$@"; do [ -f "$f" ] || continue
        $PACKT backup "$f" "$store" --similarity-threshold 0.7 > /dev/null 2>&1
    done
    local end=$(date +%s%N)
    local ms=$(( (end - start) / 1000000 ))
    local sz=$(find "$store/packs" -name '*.pack' -printf '%s\n' 2>/dev/null | awk '{sum+=$1} END{print sum}')
    echo "$sz $ms"
}

run_restic() {
    local repo="$1"; shift
    export RESTIC_REPOSITORY="$repo"; export RESTIC_PASSWORD="test"
    restic init > /dev/null 2>&1
    local start=$(date +%s%N)
    for f in "$@"; do [ -f "$f" ] || continue
        restic backup "$f" > /dev/null 2>&1
    done
    local end=$(date +%s%N)
    local ms=$(( (end - start) / 1000000 ))
    local sz=$(du -sb "$repo" | cut -f1)
    echo "$sz $ms"
}

run_kopia() {
    local repo="$1"; shift
    which kopia > /dev/null 2>&1 || { echo "0 0"; return; }
    kopia repo create --path "$repo" --no-https > /dev/null 2>&1 || true
    local start=$(date +%s%N)
    for f in "$@"; do [ -f "$f" ] || continue
        kopia snapshot create "$f" > /dev/null 2>&1 || true
    done
    local end=$(date +%s%N)
    local ms=$(( (end - start) / 1000000 ))
    local sz=$(du -sb "$repo" | cut -f1)
    kopia repo disconnect > /dev/null 2>&1 || true
    echo "$sz $ms"
}

echo -e "\n${CYAN}1. Docker cross-image (5 images)${NC}"
read P3_DK_SZ P3_DK_MS < <(run_packt "${WORK_DIR}/p3-docker" $DOCKER_FILES)
read R_DK_SZ R_DK_MS < <(run_restic "${WORK_DIR}/r-docker" $DOCKER_FILES)
DOCKER_ORIG=$(du -sb $DOCKER_FILES | awk '{total+=$1} END{print total}')
DOCKER_P3_R=$(echo "scale=2; $DOCKER_ORIG / $P3_DK_SZ" | bc)
DOCKER_R_R=$(echo "scale=2; $DOCKER_ORIG / $R_DK_SZ" | bc 2>/dev/null || echo "?")
echo "  packt: $(numfmt --to=iec-i --suffix=B $P3_DK_SZ) (${DOCKER_P3_R}x, ${P3_DK_MS}ms)"
echo "  restic: $(numfmt --to=iec-i --suffix=B $R_DK_SZ) (${DOCKER_R_R}x, ${R_DK_MS}ms)"

echo -e "\n${CYAN}2. Daily backups (10×100MB, 5% churn)${NC}"
read P3_DY_SZ P3_DY_MS < <(run_packt "${WORK_DIR}/p3-daily" $DAILY_FILES)
read R_DY_SZ R_DY_MS < <(run_restic "${WORK_DIR}/r-daily" $DAILY_FILES)
DAILY_ORIG=$(du -sb $DAILY_FILES | awk '{total+=$1} END{print total}')
DAILY_P3_R=$(echo "scale=2; $DAILY_ORIG / $P3_DY_SZ" | bc)
DAILY_R_R=$(echo "scale=2; $DAILY_ORIG / $R_DY_SZ" | bc 2>/dev/null || echo "?")
echo "  packt: $(numfmt --to=iec-i --suffix=B $P3_DY_SZ) (${DAILY_P3_R}x, ${P3_DY_MS}ms)"
echo "  restic: $(numfmt --to=iec-i --suffix=B $R_DY_SZ) (${DAILY_R_R}x, ${R_DY_MS}ms)"

read K_DY_SZ K_DY_MS < <(run_kopia "${WORK_DIR}/k-daily" $DAILY_FILES)
if [ "$K_DY_SZ" != "0" ]; then
    K_DY_R=$(echo "scale=2; $DAILY_ORIG / $K_DY_SZ" | bc 2>/dev/null || echo "?")
    echo "  kopia: $(numfmt --to=iec-i --suffix=B $K_DY_SZ) (${K_DY_R}x, ${K_DY_MS}ms)"
fi

echo -e "\n${CYAN}3. VM snapshots (4×50MB, 2% page churn)${NC}"
read P3_VM_SZ P3_VM_MS < <(run_packt "${WORK_DIR}/p3-vm" $VM_FILES)
read R_VM_SZ R_VM_MS < <(run_restic "${WORK_DIR}/r-vm" $VM_FILES)
VM_ORIG=$(du -sb $VM_FILES | awk '{total+=$1} END{print total}')
VM_P3_R=$(echo "scale=2; $VM_ORIG / $P3_VM_SZ" | bc)
VM_R_R=$(echo "scale=2; $VM_ORIG / $R_VM_SZ" | bc 2>/dev/null || echo "?")
echo "  packt: $(numfmt --to=iec-i --suffix=B $P3_VM_SZ) (${VM_P3_R}x, ${P3_VM_MS}ms)"
echo "  restic: $(numfmt --to=iec-i --suffix=B $R_VM_SZ) (${VM_R_R}x, ${R_VM_MS}ms)"

echo -e "\n${CYAN}4. Cross-session total (all ${ORIG_TOTAL} bytes combined)${NC}"
read P3_ALL_SZ P3_ALL_MS < <(run_packt "${WORK_DIR}/p3-all" $ALL_FILES)
ALL_P3_R=$(echo "scale=2; $ORIG_TOTAL / $P3_ALL_SZ" | bc 2>/dev/null || echo "?")
echo "  packt: $(numfmt --to=iec-i --suffix=B $P3_ALL_SZ) (${ALL_P3_R}x, ${P3_ALL_MS}ms)"

# ── Verify ──
echo -e "\n${YELLOW}[3/4] Verification${NC}"
echo -n "  packt verify: "
$PACKT verify "${WORK_DIR}/p3-all" 2>&1 | tail -1

# Restore
RESTORE_DIR="${WORK_DIR}/restore"
mkdir -p "$RESTORE_DIR"
$PACKT restore "${WORK_DIR}/p3-all" "$RESTORE_DIR" > /dev/null 2>&1
PASS=0; FAIL=0
for F in $ALL_FILES; do
    B=$(basename "$F"); R="$RESTORE_DIR/$B"
    if [ -f "$R" ] && cmp -s "$F" "$R"; then
        PASS=$((PASS + 1))
    else
        echo "  MISSING/CORRUPT: $B" >&2
        FAIL=$((FAIL + 1))
    fi
done
echo "  Bit-exact restore: ${GREEN}${PASS}${NC}/${YELLOW}$((PASS+FAIL))${NC}"

# ── Summary ──
echo -e "\n${CYAN}====================================================${NC}"
echo -e "${CYAN}  BENCHMARK RESULTS SUMMARY${NC}"
echo -e "${CYAN}====================================================${NC}"
echo ""

pct() { echo "scale=1; ($2 - $1) * 100 / $2" | bc 2>/dev/null || echo "?"; }
spd() { echo "scale=0; $2 * 100 / $1" | bc 2>/dev/null || echo "?"; }

printf "  %-30s %12s %12s %10s\n" "Scenario" "packt" "restic" "Advantage"
printf "  %-30s %12s %12s %10s\n" "--------" "-----" "------" "--------"
printf "  %-30s %7sx %5sms %7sx %5sms %8s\n" \
    "Docker cross-image" \
    "${DOCKER_P3_R}" "${P3_DK_MS}" \
    "${DOCKER_R_R}" "${R_DK_MS}" \
    "$(pct $P3_DK_SZ $R_DK_SZ)%"
printf "  %-30s %7sx %5sms %7sx %5sms %8s\n" \
    "Daily backups" \
    "${DAILY_P3_R}" "${P3_DY_MS}" \
    "${DAILY_R_R}" "${R_DY_MS}" \
    "$(pct $P3_DY_SZ $R_DY_SZ)%"
printf "  %-30s %7sx %5sms %7sx %5sms %8s\n" \
    "VM snapshots" \
    "${VM_P3_R}" "${P3_VM_MS}" \
    "${VM_R_R}" "${R_VM_MS}" \
    "$(pct $P3_VM_SZ $R_VM_SZ)%"

echo ""
echo "  Cross-session total: ${ALL_P3_R}x in ${P3_ALL_MS}ms"
echo "  Bit-exact files: ${PASS}/${PASS+FAIL}"
echo ""

rm -rf "$WORK_DIR"