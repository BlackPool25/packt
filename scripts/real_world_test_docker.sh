#!/usr/bin/env bash
set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'

IMAGE="${1:-ubuntu}"
VERSIONS=("22.04" "23.04" "23.10" "24.04")
WORK_DIR=$(mktemp -d)
STORE_DIR="${WORK_DIR}/store"
LAYERS_DIR="${WORK_DIR}/layers"
COMPRESSOR_BIN="${CARGO_TARGET_DIR:-target}/release/packt-cli"

echo -e "${CYAN}=== Real-World Docker Layer Sieve Test ===${NC}"
echo "Image:        $IMAGE"
echo "Versions:     ${VERSIONS[*]}"
echo ""

[ ! -f "$COMPRESSOR_BIN" ] && cargo build --release -p packt-cli 2>&1 | tail -1

mkdir -p "$STORE_DIR" "$LAYERS_DIR"
TOTAL_ORIG_SIZE=0

if command -v docker &> /dev/null; then
    for VER in "${VERSIONS[@]}"; do
        TAG="${IMAGE}:${VER}"
        VER_DIR="${LAYERS_DIR}/${VER}"
        mkdir -p "$VER_DIR"

        echo -n "Pulling $TAG... "
        docker pull "$TAG" > /dev/null 2>&1 && echo "ok" || { echo "failed"; continue; }

        docker save "$TAG" > "${WORK_DIR}/${VER}.tar"

        echo -n "  Extracting layers... "
        tar -xf "${WORK_DIR}/${VER}.tar" -C "${WORK_DIR}"

        # Determine layer blobs from manifest.json
        if [ -f "${WORK_DIR}/manifest.json" ]; then
            LAYER_BLOBS=$(python3 -c "
import json
with open('${WORK_DIR}/manifest.json') as f:
    manifest = json.load(f)
for entry in manifest:
    for lp in entry.get('Layers', []):
        print(lp)
" 2>/dev/null) || LAYER_BLOBS=""
        fi

        if [ -z "$LAYER_BLOBS" ]; then
            # Heuristic: use any blob that is a tar archive (not JSON config)
            for BLOB in "${WORK_DIR}"/blobs/sha256/*; do
                ft=$(file -b "$BLOB")
                if [[ "$ft" == *"tar archive"* ]] || [[ "$ft" == *"POSIX tar"* ]]; then
                    LAYER_BLOBS="$LAYER_BLOBS blobs/sha256/$(basename $BLOB)"
                fi
            done
        fi

        LAYER_NUM=0
        for BLOB_PATH in $LAYER_BLOBS; do
            BLOB_FILE="${WORK_DIR}/${BLOB_PATH}"
            if [ -f "$BLOB_FILE" ]; then
                cp "$BLOB_FILE" "${VER_DIR}/layer_${LAYER_NUM}.tar"
                LAYER_NUM=$((LAYER_NUM + 1))
            fi
        done

        SIZE=$(du -sb "$VER_DIR" | cut -f1)
        TOTAL_ORIG_SIZE=$((TOTAL_ORIG_SIZE + SIZE))
        echo "$LAYER_NUM layers, $(numfmt --to=iec-i --suffix=B $SIZE)"
    done
else
    for i in "${!VERSIONS[@]}"; do
        VER_DIR="${LAYERS_DIR}/${VERSIONS[$i]}"; mkdir -p "$VER_DIR"
        dd if=/dev/urandom of="${VER_DIR}/base.bin" bs=1M count=50 2>/dev/null
        dd if=/dev/urandom of="${VER_DIR}/unique_${i}.bin" bs=1M count=$((10 + i*5)) 2>/dev/null
        SIZE=$(du -sb "$VER_DIR" | cut -f1); TOTAL_ORIG_SIZE=$((TOTAL_ORIG_SIZE + SIZE))
        echo "  ${VERSIONS[$i]}: $(numfmt --to=iec-i --suffix=B $SIZE) (synthetic)"
    done
fi

echo -e "\n${CYAN}Running packt backup...${NC}"
START_TIME=$(date +%s%N)
for VER in "${VERSIONS[@]}"; do
    VER_DIR="${LAYERS_DIR}/${VER}"
    [ -d "$VER_DIR" ] || continue
    [ "$(find "$VER_DIR" -type f 2>/dev/null | wc -l)" -eq 0 ] && continue
    echo -n "  ${VER}: "
    cat "${VER_DIR}"/*.tar > "${WORK_DIR}/${VER}.bin" 2>/dev/null
    if [ -f "${WORK_DIR}/${VER}.bin" ]; then
        $COMPRESSOR_BIN backup "${WORK_DIR}/${VER}.bin" "$STORE_DIR" 2>&1 | tail -1
    fi
done
END_TIME=$(date +%s%N)
DURATION_MS=$(( (END_TIME - START_TIME) / 1000000 ))

echo -e "\n${CYAN}Store info:${NC}" && $COMPRESSOR_BIN info "$STORE_DIR"
echo -e "\n${CYAN}Verifying:${NC}" && $COMPRESSOR_BIN verify "$STORE_DIR" && echo -e "${GREEN}  OK${NC}"

echo -e "\n${CYAN}Results:${NC}"
STORE_SIZE=$(du -sb "$STORE_DIR" | cut -f1)
echo "  Original total: $(numfmt --to=iec-i --suffix=B $TOTAL_ORIG_SIZE)"
echo "  Stored:         $(numfmt --to=iec-i --suffix=B $STORE_SIZE)"
if [ "$TOTAL_ORIG_SIZE" -gt 0 ] && [ "$STORE_SIZE" -gt 0 ]; then
    RATIO=$(echo "scale=2; $TOTAL_ORIG_SIZE / $STORE_SIZE" | bc 2>/dev/null || echo "0")
    SAVINGS=$(echo "scale=1; (1 - $STORE_SIZE.0 / $TOTAL_ORIG_SIZE) * 100" | bc 2>/dev/null || echo "0")
    echo "  Sieve ratio:    ${RATIO}x"
    echo "  Space savings:  ${SAVINGS}%"
fi
echo "  Duration:       ${DURATION_MS}ms"
rm -rf "$WORK_DIR"
echo -e "${GREEN}Done${NC}"
