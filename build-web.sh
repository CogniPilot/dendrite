#!/bin/bash
set -e

echo "Building WASM..."
cargo build --target wasm32-unknown-unknown -p dendrite-web --release

echo "Running wasm-bindgen..."
~/.cargo/bin/wasm-bindgen --target web --out-dir web/pkg --out-name dendrite_web target/wasm32-unknown-unknown/release/dendrite_web.wasm

echo "Build complete. WASM output:"
ls -la web/pkg/dendrite_web_bg.wasm
