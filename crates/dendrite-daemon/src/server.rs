//! Web server setup and routing

use anyhow::Result;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::info;

use crate::api;
use crate::config::TlsConfig;
use crate::state::AppState;
use crate::ws;

/// Run the web server (HTTP or HTTPS depending on config)
pub async fn run(state: Arc<AppState>, bind: &str, tls: Option<&TlsConfig>) -> Result<()> {
    // Build router
    let app = Router::new()
        // API routes
        .route("/api/devices", get(api::list_devices))
        .route("/api/devices/{id}", get(api::get_device))
        .route("/api/devices/{id}/query", post(api::query_device))
        .route("/api/topology", get(api::get_topology))
        .route("/api/hcdf", get(api::get_hcdf))
        .route("/api/hcdf", post(api::save_hcdf))
        .route("/api/scan", post(api::trigger_scan))
        .route("/api/devices/{id}", delete(api::remove_device))
        .route("/api/config", get(api::get_config))
        .route("/api/interfaces", get(api::list_interfaces))
        .route("/api/subnet", post(api::update_subnet))
        // WebSocket for real-time updates
        .route("/ws", get(ws::websocket_handler))
        // Serve models
        .nest_service("/models", ServeDir::new(&state.config.models.path))
        // Static files (WASM frontend) - must be fallback for root
        .fallback_service(ServeDir::new("web"))
        // CORS
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        // State
        .with_state(state.clone());

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
