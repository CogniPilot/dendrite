//! Authentication middleware for token validation
//!
//! This module provides middleware for validating session tokens issued by
//! dendrite-se051d after NFC authentication with the SE051C2 secure element.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, trace, warn};

use crate::config::AuthConfig;

/// Shared token store format (must match dendrite-se051d's format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedTokenStore {
    /// Version for compatibility
    pub version: u32,
    /// Unix timestamp when store was last updated
    pub updated_at: u64,
    /// Active sessions
    pub sessions: Vec<SharedSession>,
}

/// Session info from token store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSession {
    /// Token as hex string
    pub token: String,
    /// Phone identifier as hex
    pub phone_id: String,
    /// Phone display name
    pub phone_name: String,
    /// Unix timestamp when session expires
    pub expires_at: u64,
    /// Whether connected via AP mode
    pub via_ap: bool,
}

impl SharedTokenStore {
    /// Create empty store
    pub fn new() -> Self {
        Self {
            version: 1,
            updated_at: 0,
            sessions: Vec::new(),
        }
    }

    /// Load from file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })
    }

    /// Check if a token is valid (exists and not expired)
    pub fn is_token_valid(&self, token_hex: &str) -> bool {
        let now = current_unix_time();
        self.sessions.iter().any(|s| s.token == token_hex && s.expires_at > now)
    }

    /// Get session info for a token
    pub fn get_session(&self, token_hex: &str) -> Option<&SharedSession> {
        let now = current_unix_time();
        self.sessions.iter().find(|s| s.token == token_hex && s.expires_at > now)
    }
}

impl Default for SharedTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current Unix timestamp
fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Authentication state that watches the token store file
pub struct AuthState {
    config: AuthConfig,
    store: RwLock<SharedTokenStore>,
    last_load: RwLock<SystemTime>,
}

impl AuthState {
    /// Create new auth state
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config,
            store: RwLock::new(SharedTokenStore::new()),
            last_load: RwLock::new(SystemTime::UNIX_EPOCH),
        }
    }

    /// Check if authentication is required
    pub fn is_required(&self) -> bool {
        self.config.require_token
    }

    /// Reload token store if file has changed (checks every 2 seconds)
    async fn maybe_reload(&self) {
        let now = SystemTime::now();
        let last = *self.last_load.read().await;

        // Only check file every 2 seconds
        if now.duration_since(last).unwrap_or(Duration::ZERO) < Duration::from_secs(2) {
            return;
        }

        let path = Path::new(&self.config.token_store_path);
        if !path.exists() {
            trace!("Token store file not found: {}", self.config.token_store_path);
            return;
        }

        // Check file modification time
        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(modified) = metadata.modified() {
                let store_updated = self.store.read().await.updated_at;
                let file_mtime = modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_secs();

                // Reload if file is newer
                if file_mtime > store_updated {
                    if let Ok(new_store) = SharedTokenStore::load(path) {
                        debug!(
                            sessions = new_store.sessions.len(),
                            "Reloaded token store"
                        );
                        *self.store.write().await = new_store;
                    }
                }
            }
        }

        *self.last_load.write().await = now;
    }

    /// Validate a token
    pub async fn validate_token(&self, token: &str) -> bool {
        self.maybe_reload().await;
        self.store.read().await.is_token_valid(token)
    }

    /// Get session info for a token
    pub async fn get_session(&self, token: &str) -> Option<SharedSession> {
        self.maybe_reload().await;
        self.store.read().await.get_session(token).cloned()
    }
}

/// Error response for authentication failures
#[derive(Serialize)]
struct AuthError {
    error: String,
    code: &'static str,
}

/// Authentication middleware
///
/// Validates Bearer tokens from the Authorization header when auth is required.
/// Passes through all requests when auth is disabled (development mode).
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Response {
    // If auth not required, pass through
    if !state.is_required() {
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(header) if header.starts_with("Bearer ") => &header[7..],
        Some(_) => {
            warn!("Invalid authorization header format");
            return (
                StatusCode::UNAUTHORIZED,
                Json(AuthError {
                    error: "Invalid authorization header format. Use: Bearer <token>".to_string(),
                    code: "INVALID_AUTH_FORMAT",
                }),
            )
                .into_response();
        }
        None => {
            debug!(path = %request.uri().path(), "Missing authorization header");
            return (
                StatusCode::UNAUTHORIZED,
                Json(AuthError {
                    error: "Authorization required. Include header: Authorization: Bearer <token>".to_string(),
                    code: "AUTH_REQUIRED",
                }),
            )
                .into_response();
        }
    };

    // Validate token
    if !state.validate_token(token).await {
        warn!("Invalid or expired token");
        return (
            StatusCode::UNAUTHORIZED,
            Json(AuthError {
                error: "Invalid or expired token".to_string(),
                code: "INVALID_TOKEN",
            }),
        )
            .into_response();
    }

    // Token valid, proceed
    debug!("Token validated successfully");
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_store_validation() {
        let mut store = SharedTokenStore::new();
        let future_time = current_unix_time() + 3600; // 1 hour from now

        store.sessions.push(SharedSession {
            token: "abc123".to_string(),
            phone_id: "phone1".to_string(),
            phone_name: "Test Phone".to_string(),
            expires_at: future_time,
            via_ap: false,
        });

        assert!(store.is_token_valid("abc123"));
        assert!(!store.is_token_valid("invalid"));
    }

    #[test]
    fn test_expired_token() {
        let mut store = SharedTokenStore::new();
        let past_time = current_unix_time() - 3600; // 1 hour ago

        store.sessions.push(SharedSession {
            token: "expired123".to_string(),
            phone_id: "phone1".to_string(),
            phone_name: "Test Phone".to_string(),
            expires_at: past_time,
            via_ap: false,
        });

        assert!(!store.is_token_valid("expired123"));
    }
}
