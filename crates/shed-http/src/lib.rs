//! The HTTP surface, response bytes matched to Flask (`/health` at Stage 1;
//! the browser routes join route-by-route as their stages land, each with a
//! byte-parity test against a capture from the running Python app).
//!
//! The response contract that matters to the browser: compact JSON with
//! sorted keys and a trailing newline (Flask's `jsonify`), and the exact
//! `application/json` Content-Type — Flask sends no `; charset=` suffix, so
//! neither do we.

pub mod health;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use shed_config::AppConfig;

/// Build the router over the startup configuration.
pub fn build_router(config: Arc<AppConfig>) -> Router {
    Router::new()
        .route("/health", get(health::healthcheck))
        .with_state(config)
}
