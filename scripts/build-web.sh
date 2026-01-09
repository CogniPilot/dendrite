#!/bin/bash
# Fast WASM build script for development

cd "$(dirname "$0")/.."

MODE="${1:-dev}"

case "$MODE" in
    dev|fast)
        echo "Building WASM (dev mode - fast, no optimization)..."
        cd crates/dendrite-web
        wasm-pack build --target web --dev --out-dir ../../web/pkg
        ;;
    release|prod)
        echo "Building WASM (release mode - optimized)..."
        cd crates/dendrite-web
        wasm-pack build --target web --out-dir ../../web/pkg
        ;;
    *)
        echo "Usage: $0 [dev|release]"
        echo "  dev     - Fast build for development (default)"
        echo "  release - Optimized build for production"
        exit 1
        ;;
esac
