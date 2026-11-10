//! The HTTP surface, response bytes matched to Flask (`/health` at Stage 1;
//! the page, its assets and the first `/api/` routes joined at Stage 3, each
//! with a byte-parity test against a capture from the running Python app).
//!
//! The response contract that matters to the browser: compact JSON with
//! sorted keys and a trailing newline (Flask's `jsonify`), the exact
//! `application/json` Content-Type — Flask sends no `; charset=` suffix, so
//! neither do we — and the three no-store headers on the four files the page
//! is built from.
//!
//! Two hooks wrap every route rather than appearing in each handler, because
//! that is where the source puts them:
//!
//! * [`request_id`], the `X-Request-ID` a browser complaint is grepped by
//!   (`agent.py:1015-1027`), which the router installs as middleware;
//! * [`state`], the one `Arc<AppCore>` the router hands every handler, which
//!   is where the discovered model catalog and the character caches live —
//!   the module globals of `agent.py` that its views read.
//!
//! A path no route claims answers the way Flask answers it: an empty-bodied
//! 404 from the router, not the JSON envelope. `api_json_boundary`
//! (`agent.py:1002`) wraps the *handlers*, so an undefined path never reaches
//! it, and the envelope belongs to the routes that can fail rather than to
//! the router.

// The handler modules are crate-private on purpose: what a caller builds is
// the router, never a handler. `api` in particular takes `Query<Args>`, and a
// public handler over a crate-private argument type is a type a caller can
// name in a signature but never construct. `page` stays public because the
// asset tests resolve the four frozen files through `PagePaths`, and `health`
// because `bins/shed-server --smoke` compares its bytes against the payload.
mod api;
mod asset;
pub mod health;
mod json;
pub mod page;
mod request_id;
pub mod state;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::request_id::assign_request_id;
use crate::state::AppState;

/// Build the router over the state every handler reads.
///
/// The state arrives already holding the discovered catalog: `shed-http`
/// serves what the engine stage found and never discovers anything on its own
/// except at `/api/refresh-models`, the one route the source lets rewrite it.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // The page and the three files it names (`agent.py:8418-8479`). These
        // are the only routes whose cache policy is no-store, and the only
        // ones that are not JSON.
        .route("/", get(page::index))
        .route("/app.css", get(asset::stylesheet))
        .route("/app-helpers.js", get(asset::helpers))
        .route("/styles/information_info_1565.webp", get(asset::info_icon))
        .route("/health", get(health::healthcheck))
        // The picker's three reads, the model list, the last-picked-model
        // pair, and the refresh that rewrites the catalog.
        .route("/api/countries", get(api::countries))
        .route("/api/voices", get(api::voices))
        .route("/api/characters", get(api::characters))
        .route("/api/models", get(api::models))
        .route("/api/get-last-model-index", get(api::last_model_index))
        .route("/api/set-last-model-index", post(api::set_last_model_index))
        .route("/api/refresh-models", post(api::refresh_models))
        // `before_request`/`after_request` (`agent.py:1015-1027`) are global,
        // and each one decides for itself whether the path starts with
        // `/api/`; one middleware that does the same test is the same pair.
        .layer(middleware::from_fn(assign_request_id))
        .with_state(state)
}
