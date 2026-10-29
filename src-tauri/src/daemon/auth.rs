//! API key authentication middleware for daemon mode

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;
use std::sync::Arc;

use super::scheduler::AppState;

/// Middleware that checks the `X-API-Key` header against the configured key.
/// - Skips auth for `/api/health` (public endpoint)
/// - Skips auth for `/api/gcp/token` from loopback (local service JVMs)
/// - If no API key is configured (empty string), all requests pass (dev mode)
pub async fn require_api_key(
    state: axum::extract::State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Health endpoint is always public
    if request.uri().path() == "/api/health" {
        return Ok(next.run(request).await);
    }

    // The token broker answers this server only (the handler re-checks). Sites
    // set up before nucleus have no PURU_TOKEN_KEY in their compose, so their
    // pacs/realtime/comm call it without a key.
    if request.uri().path() == "/api/gcp/token" {
        let loopback = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if loopback {
            return Ok(next.run(request).await);
        }
    }

    // No API key configured → dev mode, skip auth
    if state.api_key.is_empty() {
        return Ok(next.run(request).await);
    }

    // Check the X-API-Key header
    let provided = request
        .headers()
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok());

    match provided {
        Some(key) if key == state.api_key => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
