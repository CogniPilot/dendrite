#!/bin/bash
# Build script for dendrite-viewer WASM (standalone HCDF viewer)
# Builds both WebGPU and WebGL2 versions for runtime fallback

set -e
cd "$(dirname "$0")/.."

MODE="${1:-release}"

case "$MODE" in
    dev|fast)
        RELEASE_FLAG=""
        WASM_DIR="debug"
        echo "Building dendrite-viewer WASM (dev mode - fast, no optimization)..."
        ;;
    release|prod)
        RELEASE_FLAG="--release"
        WASM_DIR="release"
        echo "Building dendrite-viewer WASM (release mode - optimized)..."
        ;;
    *)
        echo "Usage: $0 [dev|release]"
        echo "  dev     - Fast build for development"
        echo "  release - Optimized build for production (default)"
        exit 1
        ;;
esac

# Build WebGPU version
echo ""
echo "=== Building WebGPU version ==="
cargo build --target wasm32-unknown-unknown -p dendrite-viewer $RELEASE_FLAG --no-default-features --features webgpu

# Run wasm-bindgen for WebGPU
echo "Running wasm-bindgen for WebGPU..."
~/.cargo/bin/wasm-bindgen --target web --out-dir viewer-web/pkg --out-name dendrite_viewer_webgpu "target/wasm32-unknown-unknown/$WASM_DIR/dendrite_viewer.wasm"

# Build WebGL2 version
echo ""
echo "=== Building WebGL2 version ==="
cargo build --target wasm32-unknown-unknown -p dendrite-viewer $RELEASE_FLAG --no-default-features --features webgl2

# Run wasm-bindgen for WebGL2
echo "Running wasm-bindgen for WebGL2..."
~/.cargo/bin/wasm-bindgen --target web --out-dir viewer-web/pkg --out-name dendrite_viewer_webgl2 "target/wasm32-unknown-unknown/$WASM_DIR/dendrite_viewer.wasm"

echo ""
echo "=== Build complete ==="
echo "Output: viewer-web/pkg/"
ls -la viewer-web/pkg/dendrite_viewer_webgpu_bg.wasm viewer-web/pkg/dendrite_viewer_webgl2_bg.wasm 2>/dev/null || true
echo ""
echo "To serve locally: python3 -m http.server 8081 --directory viewer-web"
