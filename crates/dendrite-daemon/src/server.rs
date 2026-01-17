//! Web server setup and routing

use anyhow::Result;
use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::info;

use crate::api;
use crate::auth::{self, AuthState};
use crate::config::TlsConfig;
use crate::state::AppState;
use crate::ws;

/// Run the web server (HTTP or HTTPS depending on config)
pub async fn run(state: Arc<AppState>, bind: &str, tls: Option<&TlsConfig>) -> Result<()> {
    // Get the cached models directory from the HCDF fetcher
    let cached_models_dir = state.hcdf_fetcher.models_dir().await;
    info!(
        static_models = %state.config.models.path,
        cached_models = %cached_models_dir.display(),
        "Serving models from static and cached directories"
    );

    // Initialize authentication state
    let auth_state = Arc::new(AuthState::new(state.config.auth.clone()));
    info!(
        require_token = state.config.auth.require_token,
        token_store = %state.config.auth.token_store_path,
        "Authentication configured"
    );

    // Build API router with optional auth middleware
    let api_router = Router::new()
        .route("/devices", get(api::list_devices))
        .route("/devices/{id}", get(api::get_device))
        .route("/devices/{id}/query", post(api::query_device))
        .route("/topology", get(api::get_topology))
        .route("/hcdf", get(api::get_hcdf))
        .route("/hcdf", post(api::save_hcdf))
        .route("/scan", post(api::trigger_scan))
        .route("/devices/{id}", delete(api::remove_device))
        .route("/config", get(api::get_config))
        .route("/interfaces", get(api::list_interfaces))
        .route("/subnet", post(api::update_subnet))
        .route("/heartbeat", get(api::get_heartbeat))
        .route("/heartbeat", post(api::set_heartbeat))
        // Device position updates
        .route("/devices/{id}/position", put(api::update_device_position))
        // Firmware checking
        .route("/firmware/check", get(api::check_all_firmware))
        .route("/firmware/{id}/check", get(api::check_firmware))
        // OTA firmware updates
        .route("/ota", get(api::get_all_ota_updates))
        .route("/ota/{id}/start", post(api::start_ota_update))
        .route("/ota/{id}/progress", get(api::get_ota_progress))
        .route("/ota/{id}/cancel", post(api::cancel_ota_update))
        .route("/ota/{id}/upload-local", post(api::upload_local_firmware))
        // HCDF import/export (for file picker)
        .route("/hcdf/export", get(api::export_hcdf))
        .route("/hcdf/import", post(api::import_hcdf))
        .route("/hcdf/save", post(api::save_hcdf_to_server))
        .with_state(state.clone())
        // Apply auth middleware to all API routes
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth::auth_middleware,
        ));

    // Build main router
    let app = Router::new()
        // Nest API routes under /api
        .nest("/api", api_router)
        // WebSocket for real-time updates (no auth - uses token in message)
        .route("/ws", get(ws::websocket_handler))
        .with_state(state.clone())
        // Serve cached models (from remote HCDF fetch) - takes precedence
        .nest_service("/models", ServeDir::new(&cached_models_dir)
            .fallback(ServeDir::new(&state.config.models.path)))
        // Static files (WASM frontend) - must be fallback for root
        .fallback_service(ServeDir::new("web"))
        // CORS
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Start discovery in background
    let scanner = state.scanner.clone();
    tokio::spawn(async move {
        if let Err(e) = scanner.run().await {
            tracing::error!(error = %e, "Discovery scanner failed");
        }
    });

    // Start server with or without TLS
    if let Some(tls_config) = tls {
        run_https(app, bind, tls_config).await
    } else {
        run_http(app, bind).await
    }
}

/// Run plain HTTP server
async fn run_http(app: Router, bind: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(address = %bind, protocol = "HTTP", "Starting web server");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Run HTTPS server with TLS
async fn run_https(app: Router, bind: &str, tls: &TlsConfig) -> Result<()> {
    use axum_server::tls_rustls::RustlsConfig;
    use std::path::PathBuf;

    let cert_path = PathBuf::from(&tls.cert);
    let key_path = PathBuf::from(&tls.key);

    // Verify files exist
    if !cert_path.exists() {
        anyhow::bail!("TLS certificate file not found: {}", tls.cert);
    }
    if !key_path.exists() {
        anyhow::bail!("TLS key file not found: {}", tls.key);
    }

    let rustls_config = RustlsConfig::from_pem_file(&cert_path, &key_path).await?;

    let addr: std::net::SocketAddr = bind.parse()?;
    info!(address = %bind, protocol = "HTTPS", cert = %tls.cert, "Starting web server with TLS");

    axum_server::bind_rustls(addr, rustls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
