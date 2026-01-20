//! Dendrite Viewer - Standalone HCDF visualization
//!
//! A lightweight 3D viewer for HCDF (Hardware Configuration Description Format) files.
//! Based on dendrite-web but without network scanning and firmware features.

mod app;
mod file_picker;
mod models;
mod scene;
mod ui;

use wasm_bindgen::prelude::*;

/// WASM entry point
#[wasm_bindgen(start)]
pub fn main() {
    // Set up panic hook for better error messages
    console_error_panic_hook::set_once();

    // Initialize logging with filtering to reduce noise
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::WARN)
            .build()
    );

    // Run the Bevy app
    app::run();
}
