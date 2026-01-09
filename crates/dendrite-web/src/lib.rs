//! Dendrite Web - WebGPU-powered 3D visualization frontend
//!
//! This crate provides the browser-based visualization using Bevy and WebGPU.

mod app;
mod models;
mod network;
mod scene;
mod ui;

use wasm_bindgen::prelude::*;

/// Entry point for WASM module
#[wasm_bindgen(start)]
pub fn main() {
    // Set panic hook for better error messages
    console_error_panic_hook::set_once();

    // Initialize logging with filtering to reduce wgpu noise
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::WARN)
            .build()
    );

    // Run the Bevy app
    app::run();
}
