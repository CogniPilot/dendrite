//! Reusable file picker system for WASM
//!
//! Provides a modal-based file picker that can be used for:
//! - Firmware upload (pick .bin files)
//! - HCDF import/export (pick/save .hcdf files)
//! - Any future file operations
//!
//! Uses JavaScript interop for native file dialogs in the browser.

use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// File picker plugin
pub struct FilePickerPlugin;

impl Plugin for FilePickerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FilePickerState>()
            .init_resource::<PendingFileResults>()
            .add_systems(Update, process_file_results);
    }
}

/// Type of file operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    /// Pick a file to open/upload
    Open,
    /// Save content to a file
    Save,
}

/// Context for what the file picker is being used for
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePickerContext {
    /// Uploading firmware to a specific device
    FirmwareUpload { device_id: String },
    /// Importing an HCDF file
    HcdfImport,
    /// Exporting/saving the current HCDF
    HcdfExport,
    /// Custom context with a string identifier
    Custom(String),
}

/// File filter for the picker dialog
#[derive(Debug, Clone)]
pub struct FileFilter {
    /// Display name (e.g., "Firmware Files")
    pub name: String,
    /// File extensions without dots (e.g., ["bin", "hex"])
    pub extensions: Vec<String>,
}

impl FileFilter {
    pub fn firmware() -> Self {
        Self {
            name: "Firmware Files".to_string(),
            extensions: vec!["bin".to_string(), "hex".to_string()],
        }
    }

    pub fn hcdf() -> Self {
        Self {
            name: "HCDF Files".to_string(),
            extensions: vec!["hcdf".to_string(), "xml".to_string()],
        }
    }

    pub fn all() -> Self {
        Self {
            name: "All Files".to_string(),
            extensions: vec![],
        }
    }

    /// Convert to accept string for HTML input element
    pub fn to_accept_string(&self) -> String {
        if self.extensions.is_empty() {
            "*".to_string()
        } else {
            self.extensions
                .iter()
                .map(|ext| format!(".{}", ext))
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}

/// Request to open the file picker
#[derive(Debug, Clone)]
pub struct FilePickerRequest {
    /// What operation to perform
    pub operation: FileOperation,
    /// Context for the operation (what we're picking for)
    pub context: FilePickerContext,
    /// File filter
    pub filter: FileFilter,
    /// Default filename (for save operations)
    pub default_filename: Option<String>,
    /// Content to save (for save operations)
    pub save_content: Option<Vec<u8>>,
}

/// Result from a file picker operation
#[derive(Debug, Clone)]
pub struct FilePickerResult {
    /// The context this result is for
    pub context: FilePickerContext,
    /// The operation that was performed
    pub operation: FileOperation,
    /// Filename (without path)
    pub filename: String,
    /// File content (for open operations)
    pub content: Option<Vec<u8>>,
    /// Whether the operation succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Pending file results from JavaScript callbacks
#[derive(Resource, Default)]
pub struct PendingFileResults(pub Arc<Mutex<VecDeque<FilePickerResult>>>);

/// File picker state
#[derive(Resource, Default)]
pub struct FilePickerState {
    /// Whether a file picker is currently open
    pub is_open: bool,
    /// Current pending request (waiting for user interaction)
    pub current_request: Option<FilePickerRequest>,
    /// Completed results ready for processing
    pub completed_results: VecDeque<FilePickerResult>,
}

impl FilePickerState {
    /// Request to open a file
    pub fn request_open(&mut self, context: FilePickerContext, filter: FileFilter) {
        self.current_request = Some(FilePickerRequest {
            operation: FileOperation::Open,
            context,
            filter,
            default_filename: None,
            save_content: None,
        });
        self.is_open = true;
    }

    /// Request to save a file
    pub fn request_save(
        &mut self,
        context: FilePickerContext,
        filter: FileFilter,
        filename: String,
        content: Vec<u8>,
    ) {
        self.current_request = Some(FilePickerRequest {
            operation: FileOperation::Save,
            context,
            filter,
            default_filename: Some(filename),
            save_content: Some(content),
        });
        self.is_open = true;
    }

    /// Take the next completed result
    pub fn take_result(&mut self) -> Option<FilePickerResult> {
        self.completed_results.pop_front()
    }

    /// Check if there are pending results for a specific context
    pub fn has_result_for(&self, context: &FilePickerContext) -> bool {
        self.completed_results.iter().any(|r| &r.context == context)
    }

    /// Close the picker (cancel)
    pub fn close(&mut self) {
        self.is_open = false;
        self.current_request = None;
    }
}

/// System to process file results from JavaScript callbacks
fn process_file_results(
    pending: Res<PendingFileResults>,
    mut picker_state: ResMut<FilePickerState>,
) {
    // Move results from pending (JS callback) to completed (ready for UI)
    if let Ok(mut pending_results) = pending.0.lock() {
        while let Some(result) = pending_results.pop_front() {
            picker_state.completed_results.push_back(result);
            picker_state.is_open = false;
            picker_state.current_request = None;
        }
    }
}

// ============================================================================
// JavaScript Interop (WASM only)
// ============================================================================

#[cfg(target_arch = "wasm32")]
mod js_interop {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{HtmlInputElement, Blob, Url};

    /// Open a file picker dialog using HTML input element
    pub fn open_file_picker(
        accept: &str,
        pending_results: Arc<Mutex<VecDeque<FilePickerResult>>>,
        context: FilePickerContext,
    ) {
        tracing::warn!("open_file_picker: starting, accept={}", accept);

        let window = match web_sys::window() {
            Some(w) => w,
            None => {
                tracing::error!("open_file_picker: no window object");
                return;
            }
        };
        let document = match window.document() {
            Some(d) => d,
            None => {
                tracing::error!("open_file_picker: no document object");
                return;
            }
        };

        // Create a hidden file input element
        let input: HtmlInputElement = match document.create_element("input") {
            Ok(el) => match el.dyn_into::<HtmlInputElement>() {
                Ok(input) => input,
                Err(_) => {
                    tracing::error!("open_file_picker: failed to cast to HtmlInputElement");
                    return;
                }
            },
            Err(e) => {
                tracing::error!("open_file_picker: failed to create input element: {:?}", e);
                return;
            }
        };

        input.set_type("file");
        input.set_accept(accept);
        input.style().set_property("display", "none").ok();

        // Append to body temporarily
        if let Some(body) = document.body() {
            if let Err(e) = body.append_child(&input) {
                tracing::error!("open_file_picker: failed to append input to body: {:?}", e);
                return;
            }
            tracing::warn!("open_file_picker: input element appended to body");
        } else {
            tracing::error!("open_file_picker: no document body");
            return;
        }

        // Set up change handler
        let input_clone = input.clone();
        let pending_clone = pending_results.clone();
        let context_clone = context.clone();

        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            tracing::warn!("open_file_picker: change event fired");
            let files = input_clone.files();
            if let Some(files) = files {
                tracing::warn!("open_file_picker: got {} files", files.length());
                if files.length() > 0 {
                    if let Some(file) = files.get(0) {
                        let filename = file.name();
                        tracing::warn!("open_file_picker: reading file: {}", filename);
                        let pending = pending_clone.clone();
                        let ctx = context_clone.clone();

                        // Read file content
                        let reader = web_sys::FileReader::new().unwrap();
                        let reader_clone = reader.clone();

                        let onload = Closure::wrap(Box::new(move |_: web_sys::Event| {
                            tracing::warn!("open_file_picker: file read complete");
                            let result = reader_clone.result().unwrap();
                            let array_buffer = result.dyn_into::<js_sys::ArrayBuffer>().unwrap();
                            let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                            let content = uint8_array.to_vec();

                            if let Ok(mut results) = pending.lock() {
                                results.push_back(FilePickerResult {
                                    context: ctx.clone(),
                                    operation: FileOperation::Open,
                                    filename: filename.clone(),
                                    content: Some(content),
                                    success: true,
                                    error: None,
                                });
                                tracing::warn!("open_file_picker: result pushed to pending queue");
                            }
                        }) as Box<dyn FnMut(_)>);

                        reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                        onload.forget();

                        reader.read_as_array_buffer(&file).ok();
                    }
                }
            } else {
                tracing::warn!("open_file_picker: no files selected");
            }

            // Remove the input element
            if let Some(parent) = input_clone.parent_node() {
                parent.remove_child(&input_clone).ok();
            }
        }) as Box<dyn FnMut(_)>);

        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        closure.forget();

        // Trigger the file picker
        tracing::warn!("open_file_picker: calling input.click()");
        input.click();
        tracing::warn!("open_file_picker: input.click() returned");
    }

    /// Save content to a file using download
    pub fn save_file(
        filename: &str,
        content: &[u8],
        mime_type: &str,
        pending_results: Arc<Mutex<VecDeque<FilePickerResult>>>,
        context: FilePickerContext,
    ) {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return,
        };
        let document = match window.document() {
            Some(d) => d,
            None => return,
        };

        // Create blob from content
        let uint8_array = js_sys::Uint8Array::from(content);
        let array = js_sys::Array::new();
        array.push(&uint8_array.buffer());

        let blob_options = web_sys::BlobPropertyBag::new();
        blob_options.set_type(mime_type);

        let blob = match Blob::new_with_u8_array_sequence_and_options(&array, &blob_options) {
            Ok(b) => b,
            Err(_) => return,
        };

        // Create download URL
        let url = match Url::create_object_url_with_blob(&blob) {
            Ok(u) => u,
            Err(_) => return,
        };

        // Create temporary anchor element for download
        let anchor = match document.create_element("a") {
            Ok(el) => el,
            Err(_) => return,
        };

        anchor.set_attribute("href", &url).ok();
        anchor.set_attribute("download", filename).ok();

        if let Some(body) = document.body() {
            body.append_child(&anchor).ok();

            // Trigger download
            if let Some(html_el) = anchor.dyn_ref::<web_sys::HtmlElement>() {
                html_el.click();
            }

            body.remove_child(&anchor).ok();
        }

        // Revoke URL after a delay
        let url_clone = url.clone();
        let closure = Closure::wrap(Box::new(move || {
            Url::revoke_object_url(&url_clone).ok();
        }) as Box<dyn FnMut()>);

        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                1000,
            )
            .ok();
        closure.forget();

        // Report success
        if let Ok(mut results) = pending_results.lock() {
            results.push_back(FilePickerResult {
                context,
                operation: FileOperation::Save,
                filename: filename.to_string(),
                content: None,
                success: true,
                error: None,
            });
        }
    }
}

// Non-WASM stubs
#[cfg(not(target_arch = "wasm32"))]
mod js_interop {
    use super::*;

    pub fn open_file_picker(
        _accept: &str,
        pending_results: Arc<Mutex<VecDeque<FilePickerResult>>>,
        context: FilePickerContext,
    ) {
        // On native, we'd use rfd or similar - for now just report not supported
        if let Ok(mut results) = pending_results.lock() {
            results.push_back(FilePickerResult {
                context,
                operation: FileOperation::Open,
                filename: String::new(),
                content: None,
                success: false,
                error: Some("File picker not supported on this platform".to_string()),
            });
        }
    }

    pub fn save_file(
        filename: &str,
        _content: &[u8],
        _mime_type: &str,
        pending_results: Arc<Mutex<VecDeque<FilePickerResult>>>,
        context: FilePickerContext,
    ) {
        if let Ok(mut results) = pending_results.lock() {
            results.push_back(FilePickerResult {
                context,
                operation: FileOperation::Save,
                filename: filename.to_string(),
                content: None,
                success: false,
                error: Some("File save not supported on this platform".to_string()),
            });
        }
    }
}

// Re-export the interop functions
pub use js_interop::{open_file_picker, save_file};

/// Helper to trigger file open from UI
pub fn trigger_file_open(
    pending: &PendingFileResults,
    context: FilePickerContext,
    filter: FileFilter,
) {
    let accept = filter.to_accept_string();
    tracing::warn!("trigger_file_open: accept={}, context={:?}", accept, context);
    open_file_picker(&accept, pending.0.clone(), context);
}

/// Helper to trigger file save from UI
pub fn trigger_file_save(
    pending: &PendingFileResults,
    context: FilePickerContext,
    filename: &str,
    content: &[u8],
    mime_type: &str,
) {
    save_file(filename, content, mime_type, pending.0.clone(), context);
}
