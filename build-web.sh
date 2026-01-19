#!/bin/bash
set -e

# Build both WebGPU and WebGL2 versions for runtime fallback

echo "Building WebGPU WASM..."
cargo build --target wasm32-unknown-unknown -p dendrite-web --release --no-default-features --features webgpu

echo "Running wasm-bindgen for WebGPU..."
~/.cargo/bin/wasm-bindgen --target web --out-dir web/pkg --out-name dendrite_web_webgpu target/wasm32-unknown-unknown/release/dendrite_web.wasm

echo "Building WebGL2 WASM..."
cargo build --target wasm32-unknown-unknown -p dendrite-web --release --no-default-features --features webgl2

echo "Running wasm-bindgen for WebGL2..."
~/.cargo/bin/wasm-bindgen --target web --out-dir web/pkg --out-name dendrite_web_webgl2 target/wasm32-unknown-unknown/release/dendrite_web.wasm

echo "Build complete. WASM outputs:"
ls -la web/pkg/dendrite_web_webgpu_bg.wasm web/pkg/dendrite_web_webgl2_bg.wasm
