#!/usr/bin/env bash
set -euo pipefail

# Phase 4c Cross-Layer Delta Verification Benchmark
# Tests Palantir similarity detection across Docker image layer versions
# with the new chunk config (4KB/8KB/64KB).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

echo "=== Phase 4c: Cross-Layer Delta Verification ==="
echo ""

# Images to test (Ubuntu versions spanning multiple years)
IMAGES=(
    "ubuntu:22.04"
    "ubuntu:23.04"
    "ubuntu:23.10"
    "ubuntu:24.04"
    "ubuntu:24.10"
)

STORE_DIR="/tmp/packt-benchmark-store-$$"
RESTORE_DIR="/tmp/packt-benchmark-restore-$$"
LAYER_DIR="/tmp/packt-layers-$$"

mkdir -p "$STORE_DIR" "$RESTORE_DIR" "$LAYER_DIR"

cleanup() {
    echo "Cleaning up..."
    rm -rf "$STORE_DIR" "$RESTORE_DIR" "$LAYER_DIR"
}
trap cleanup EXIT

echo "Step 1: Pulling Docker images and extracting layers..."
echo ""

for img in "${IMAGES[@]}"; do
    echo "  Pulling $img ..."
    docker pull "$img" 2>/dev/null || true
done

echo ""
echo "Step 2: Extracting unique layers per image..."
echo ""

declare -A IMAGE_LAYERS

for img in "${IMAGES[@]}"; do
    safe_name=$(echo "$img" | tr '/:' '_')
    img_dir="$LAYER_DIR/$safe_name"
    mkdir -p "$img_dir"
    
    echo "  Saving $img ..."
    docker save "$img" -o "$LAYER_DIR/${safe_name}.tar" 2>/dev/null
    
    # Extract manifest to find layer blobs
    LAYER_COUNT=0
    if command -v python3 &>/dev/null; then
        python3 -c "
import json, os, tarfile, sys

img_name = '$img'
safe = '$safe_name'
layer_dir = '$img_dir'
archive = '$LAYER_DIR/${safe_name}.tar'

with tarfile.open(archive) as tf:
    # Find manifest.json
    manifest = json.loads(tf.extractfile('manifest.json').read())
    
    # Extract each layer
    for layer_info in manifest[0].get('Layers', []):
        layer_file = tf.extractfile(layer_info)
        out_path = os.path.join(layer_dir, os.path.basename(layer_info).replace('/', '_'))
        with open(out_path, 'wb') as f:
            f.write(layer_file.read())
        print(f'    Layer: {os.path.basename(layer_info)} -> {os.path.basename(out_path)} ({os.path.getsize(out_path)} bytes)')
        sys.stdout.flush()
    " 2>&1
    fi
done

echo ""
echo "Step 3: Running packt backup on all layers..."
echo ""

TOTAL_LAYERS=0
for img in "${IMAGES[@]}"; do
    safe_name=$(echo "$img" | tr '/:' '_')
    img_dir="$LAYER_DIR/$safe_name"
    
    for layer_file in "$img_dir"/*; do
        if [ -f "$layer_file" ]; then
            TOTAL_LAYERS=$((TOTAL_LAYERS + 1))
            echo "  Backing up: $safe_name/$(basename "$layer_file") ..."
            cargo run --release -- backup "$layer_file" "$STORE_DIR" --chunk-size 8k 2>&1 | tail -1
        fi
    done
done

echo ""
echo "Step 4: Store summary..."
echo ""

cargo run --release -- info "$STORE_DIR" 2>&1

echo ""
echo "Step 5: Verifying store integrity..."
echo ""

cargo run --release -- verify "$STORE_DIR" 2>&1

echo ""
echo "Step 6: Cross-layer dedup analysis..."
echo ""

# Get total source size from info
INFO_OUTPUT=$(cargo run --release -- info "$STORE_DIR" --json 2>/dev/null || true)
if [ -n "$INFO_OUTPUT" ]; then
    TOTAL_SOURCE=$(echo "$INFO_OUTPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['total_source_bytes'])")
    TOTAL_FILES=$(echo "$INFO_OUTPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['file_count'])")
    echo "  Total source bytes: $TOTAL_SOURCE"
    echo "  Total layer files: $TOTAL_FILES"
    echo "  Average layer size: $((TOTAL_SOURCE / TOTAL_FILES)) bytes"
fi

echo ""
echo "=== Benchmark Complete ==="
echo ""
