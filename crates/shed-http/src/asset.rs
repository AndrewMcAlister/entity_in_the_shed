//! The three files the page names in a URL: `app.css`, `app-helpers.js`,
//! and the information icon (`agent.py:8433-8479`).
//!
//! All three are the same shape — read the file, answer with its bytes, 404
//! when it is not there — so the shape lives once in [`file_response`] and
//! each route contributes its path and its one Content-Type.
//!
//! The Content-Type strings are what the running Flask sends, measured
//! against it rather than guessed: `send_file` derives `text/css` and
//! `application/javascript` from the extension through mimetypes, which
//! append `; charset=utf-8` to a textual type, and `image/webp` from the
//! same table with no charset because the type is not text. A client that
//! loads the helpers as a classic `<script>` is unaffected either way, but
//! one that checked the header byte-for-byte against a capture would notice.
//!
//! What the clone does not send is `send_file`'s conditional-GET machinery —
//! `Last-Modified`, `ETag`, and the 304 that follows an `If-None-Match`.
//! Werkzeug builds the ETag from the file's mtime as a float, and two
//! runtimes reading the same file do not agree on that float, so the ETag
//! could not be made byte-equal anyway; and because every one of these
//! responses says `no-store`, a conformant browser never revalidates them
//! and never sends the conditional request. Recorded in `docs/PARITY.md`.

use std::path::Path;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};

use crate::page::no_store;
use crate::state::AppState;

/// `GET /app.css` — `app_stylesheet` (`agent.py:8433-8443`).
pub async fn stylesheet(State(state): State<AppState>) -> Response<Body> {
    file_response(&state.paths.stylesheet, "text/css; charset=utf-8")
}

/// `GET /app-helpers.js` — `app_helpers` (`agent.py:8446-8466`).
pub async fn helpers(State(state): State<AppState>) -> Response<Body> {
    file_response(
        &state.paths.helpers,
        "application/javascript; charset=utf-8",
    )
}

/// `GET /styles/information_info_1565.webp` — `info_icon_asset`
/// (`agent.py:8469-8479`).
pub async fn info_icon(State(state): State<AppState>) -> Response<Body> {
    file_response(&state.paths.info_icon, "image/webp")
}

/// The bytes of `path` with the no-store trio, or the 404 the source's
/// `abort(404)` produces when the file does not exist.
///
/// A read error other than "missing" — a sharing violation, a directory in
/// the way — answers the same 404 rather than a 500: the file is not there
/// for this response either way, and the four routes' contract is that a
/// path they claim but cannot serve is a 404. The reason is logged, because
/// a frozen asset that has gone unreadable is an install problem.
fn file_response(path: &Path, content_type: &'static str) -> Response<Body> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("asset {path:?} could not be read: {error}");
            let mut missing = Response::new(Body::empty());
            *missing.status_mut() = StatusCode::NOT_FOUND;
            return missing;
        }
    };

    let mut response = Response::new(Body::from(bytes));
    let headers: &mut HeaderMap = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    no_store(headers);
    response
}
