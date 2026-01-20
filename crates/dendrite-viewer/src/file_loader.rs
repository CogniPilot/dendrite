//! HCDF file loading from URL or local file upload

use bevy::prelude::*;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{FileReader, HtmlInputElement};

use crate::app::LoadedHcdf;
use dendrite_core::hcdf::Hcdf;
use dendrite_scene::hcdf_convert::{comp_to_device_data, mcu_to_device_data};
use dendrite_scene::types::*;

/// Plugin for HCDF file loading
pub struct FileLoaderPlugin;

impl Plugin for FileLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingLoad>()
            .add_systems(Startup, check_url_parameter)
            .add_systems(Update, process_pending_loads);
    }
}

/// Pending file load operations
#[derive(Resource, Default)]
pub struct PendingLoad {
    pub data: Arc<Mutex<Option<String>>>,
    pub source: Arc<Mutex<Option<String>>>,
    pub error: Arc<Mutex<Option<String>>>,
}

/// Check URL for ?hcdf= parameter on startup
fn check_url_parameter(mut hcdf: ResMut<LoadedHcdf>, pending: Res<PendingLoad>) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };

    let location = match window.location().href() {
        Ok(href) => href,
        Err(_) => return,
    };

    // Parse URL for ?hcdf= parameter
    if let Ok(url) = web_sys::Url::new(&location) {
        let params = url.search_params();
        if let Some(hcdf_url) = params.get("hcdf") {
            tracing::info!("Loading HCDF from URL parameter: {}", hcdf_url);
            hcdf.loading = true;
            hcdf.source = Some(hcdf_url.clone());

            // Fetch the HCDF file
            let data_clone = pending.data.clone();
            let source_clone = pending.source.clone();
            let error_clone = pending.error.clone();
            let url_clone = hcdf_url.clone();

            wasm_bindgen_futures::spawn_local(async move {
                match fetch_hcdf(&url_clone).await {
                    Ok(content) => {
                        *data_clone.lock().unwrap() = Some(content);
                        *source_clone.lock().unwrap() = Some(url_clone);
                    }
                    Err(e) => {
                        *error_clone.lock().unwrap() = Some(e);
                    }
                }
            });
        }
    }
}

/// Fetch HCDF content from URL
async fn fetch_hcdf(url: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or("No window")?;

    let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("Fetch failed: {:?}", e))?;

    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "Response cast failed")?;

    if !resp.ok() {
        return Err(format!("HTTP {}: {}", resp.status(), resp.status_text()));
    }

    let text = wasm_bindgen_futures::JsFuture::from(resp.text().map_err(|_| "Failed to get text")?)
        .await
        .map_err(|e| format!("Text extraction failed: {:?}", e))?;

    text.as_string().ok_or_else(|| "Not a string".to_string())
}

/// Process pending load operations
fn process_pending_loads(mut hcdf: ResMut<LoadedHcdf>, pending: Res<PendingLoad>) {
    // Check for completed loads
    if let Ok(mut data) = pending.data.try_lock() {
        if let Some(content) = data.take() {
            let source = pending.source.lock().ok().and_then(|mut s| s.take());

            match parse_hcdf(&content) {
                Ok(devices) => {
                    hcdf.devices = devices;
                    hcdf.source = source;
                    hcdf.loading = false;
                    hcdf.error = None;
                    tracing::info!("Loaded {} devices from HCDF", hcdf.devices.len());
                }
                Err(e) => {
                    hcdf.loading = false;
                    hcdf.error = Some(e);
                }
            }
        }
    }

    // Check for errors
    if let Ok(mut error) = pending.error.try_lock() {
        if let Some(e) = error.take() {
            hcdf.loading = false;
            hcdf.error = Some(e);
        }
    }
}

/// Parse HCDF content into device data
fn parse_hcdf(content: &str) -> Result<Vec<DeviceData>, String> {
    let trimmed = content.trim();

    // Try JSON first
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return serde_json::from_str(content).map_err(|e| format!("JSON parse error: {}", e));
    }

    // Try XML (HCDF format)
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<hcdf") {
        return parse_hcdf_xml(content);
    }

    // Try TOML
    if content.contains("[device]") || content.contains("[[devices]]") {
        return Err("TOML HCDF parsing not yet implemented".to_string());
    }

    Err("Unknown HCDF format".to_string())
}

/// Parse HCDF XML format using shared conversion code
fn parse_hcdf_xml(content: &str) -> Result<Vec<DeviceData>, String> {
    web_sys::console::log_1(&format!("Parsing HCDF XML ({} bytes)", content.len()).into());

    let hcdf =
        Hcdf::from_xml(content).map_err(|e| format!("HCDF XML parse error: {:?}", e))?;

    web_sys::console::log_1(&format!("HCDF parsed: {} mcus, {} comps", hcdf.mcu.len(), hcdf.comp.len()).into());

    let mut devices = Vec::new();

    // Convert MCUs to DeviceData
    for mcu in &hcdf.mcu {
        web_sys::console::log_1(&format!("MCU: {} with {} visuals", mcu.name, mcu.visual.len()).into());
        devices.push(mcu_to_device_data(mcu));
    }

    // Convert Comps to DeviceData (except parent role)
    for comp in &hcdf.comp {
        web_sys::console::log_1(&format!("Comp: {} role={:?} with {} visuals", comp.name, comp.role, comp.visual.len()).into());
        if comp.role.as_deref() != Some("parent") {
            let device = comp_to_device_data(comp);
            web_sys::console::log_1(&format!("  -> Device {} with {} visuals", device.name, device.visuals.len()).into());
            for v in &device.visuals {
                web_sys::console::log_1(&format!("     Visual: {} model={:?}", v.name, v.model_path).into());
            }
            devices.push(device);
        }
    }

    Ok(devices)
}

/// Create a file input element for HCDF upload
pub fn create_file_picker(pending: &PendingLoad) {
    // Direct console log for debugging (bypasses tracing filter)
    web_sys::console::log_1(&"create_file_picker called".into());

    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            web_sys::console::error_1(&"create_file_picker: no window".into());
            return;
        }
    };

    let document = match window.document() {
        Some(d) => d,
        None => {
            tracing::error!("create_file_picker: no document");
            return;
        }
    };

    let input: HtmlInputElement = match document.create_element("input") {
        Ok(el) => match el.dyn_into() {
            Ok(input) => input,
            Err(_) => {
                tracing::error!("create_file_picker: failed to cast to HtmlInputElement");
                return;
            }
        },
        Err(e) => {
            tracing::error!(
                "create_file_picker: failed to create input element: {:?}",
                e
            );
            return;
        }
    };

    input.set_type("file");
    input.set_accept(".hcdf,.json,.toml,.xml");

    // Hide the input element but keep it in the DOM
    let _ = input.style().set_property("display", "none");

    // Append to body - required for click() to work in many browsers
    let body = match document.body() {
        Some(b) => b,
        None => {
            web_sys::console::error_1(&"create_file_picker: no document body".into());
            return;
        }
    };

    if let Err(e) = body.append_child(&input) {
        web_sys::console::error_1(&format!("create_file_picker: failed to append: {:?}", e).into());
        return;
    }

    web_sys::console::log_1(&"Input appended to body, calling click()".into());

    let data_clone = pending.data.clone();
    let source_clone = pending.source.clone();
    let error_clone = pending.error.clone();

    // Clone input for use in closure to remove it later
    let input_for_removal = input.clone();

    let closure = Closure::wrap(Box::new(move |event: web_sys::Event| {
        let input: HtmlInputElement = match event.target() {
            Some(t) => match t.dyn_into() {
                Ok(i) => i,
                Err(_) => return,
            },
            None => return,
        };

        // Remove the input from DOM after use
        if let Some(parent) = input_for_removal.parent_node() {
            let _ = parent.remove_child(&input_for_removal);
        }

        let files = match input.files() {
            Some(f) => f,
            None => return,
        };

        let file = match files.get(0) {
            Some(f) => f,
            None => return,
        };

        let filename = file.name();
        tracing::info!("File selected: {}", filename);

        let reader = match FileReader::new() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to create FileReader: {:?}", e);
                return;
            }
        };

        let data_inner = data_clone.clone();
        let source_inner = source_clone.clone();
        let error_inner = error_clone.clone();
        let filename_clone = filename.clone();

        let onload = Closure::wrap(Box::new(move |event: web_sys::Event| {
            let reader: FileReader = match event.target() {
                Some(t) => match t.dyn_into() {
                    Ok(r) => r,
                    Err(_) => return,
                },
                None => return,
            };

            match reader.result() {
                Ok(result) => {
                    if let Some(content) = result.as_string() {
                        tracing::info!("File loaded: {} bytes", content.len());
                        *data_inner.lock().unwrap() = Some(content);
                        *source_inner.lock().unwrap() = Some(filename_clone.clone());
                    }
                }
                Err(e) => {
                    tracing::error!("File read error: {:?}", e);
                    *error_inner.lock().unwrap() = Some(format!("Read error: {:?}", e));
                }
            }
        }) as Box<dyn FnMut(_)>);

        reader.set_onload(Some(onload.as_ref().unchecked_ref()));
        onload.forget();

        let _ = reader.read_as_text(&file);
    }) as Box<dyn FnMut(_)>);

    input.set_onchange(Some(closure.as_ref().unchecked_ref()));
    closure.forget();

    tracing::info!("Opening file picker dialog");
    input.click();
}
