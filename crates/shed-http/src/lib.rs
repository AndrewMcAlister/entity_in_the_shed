//! The HTTP surface: the routes, and the state they are given.
//!
//! Ported from `agent.py:11553` (`run_http_server`), which is where Flask's `app` is handed to
//! a server. The routes themselves are decorated onto `app` at import there; here they are
//! assembled by [`router`], so the surface is a value a test can call without opening a socket.
//!
//! Flask's error handling is a `@app.errorhandler(Exception)` that turns any uncaught exception
//! into a JSON body. The equivalent here is that a handler returns its own failure as a
//! response, which is why no route in this crate panics.

mod health;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use shed_config::AppConfig;

/// What every route is given.
///
/// The configuration is behind an `Arc` because every request needs it and none of them may
/// change it: `AppConfig` is read once at startup and the routes only report it.
#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        Self { config: Arc::new(config) }
    }
}

/// Every route this stage answers.
///
/// `GET /health` is the only one until the browser-facing routes arrive, so this is the whole
/// surface rather than a slice of it.
pub fn router(state: AppState) -> Router {
    Router::new().route("/health", get(health::handler)).with_state(state)
}
