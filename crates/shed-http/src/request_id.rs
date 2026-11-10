//! The `X-Request-ID` that ties a browser complaint to a server log line.
//!
//! In the source this is a Flask `before_request`/`after_request` pair
//! (`agent.py:1015-1027`) plus a `threading.local()` contextvar
//! (`current_request_id`, `agent.py:962-973`) that any function two calls deep
//! can read without having a request id passed to it. The clone cannot copy
//! that shape: axum handlers are futures that move across worker threads, so
//! there is no per-request thread to hang a value on. What it copies is the
//! *contract* — one id per request, chosen the same way, on the response and
//! available to the handlers — by putting the value in the request's extension
//! map, which is axum's per-request scope, and reading it back out of the
//! response in the same middleware that put it in.
//!
//! The id is `uuid4().hex`: 32 lowercase hex characters with no braces and no
//! dashes, which is what a browser sends back and what a log grep takes.

use axum::extract::{FromRequestParts, Request};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;

use crate::json::request_id_header;

/// The id for the request being served.
///
/// The three sources, in the order `current_request_id` tries them
/// (`agent.py:973-984`): whatever the middleware stored for this request, then
/// the incoming header, then a fresh one.
#[derive(Clone)]
pub struct RequestId(pub String);

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        if let Some(id) = parts.extensions.get::<Self>() {
            return Ok(Self(id.0.clone()));
        }
        Ok(Self(chosen_id(&parts.headers)))
    }
}

/// `request.headers.get("X-Request-ID", "").strip() or uuid4().hex`
/// (`agent.py:1019-1020`): the browser's id when it sent a non-blank one, a
/// fresh one otherwise.
///
/// The source truncates nothing — an id longer than a header can carry is
/// rejected by the HTTP layer before Flask sees it, in both implementations.
fn chosen_id(headers: &HeaderMap) -> String {
    headers
        .get(request_id_header())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(new_id, str::to_owned)
}

/// `str(uuid.uuid4()).replace("-", "")` — the hex form, so a header value
/// never carries a character a header would have to be quoted for.
fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// The before/after pair, as one middleware.
///
/// It only acts on `/api/`, exactly as the source's `path.startswith("/api/")`
/// guard does (`agent.py:1016`, `1024`). The page and its assets are not
/// diagnosed this way and Flask does not put the header on them, so neither
/// does the clone: a browser comparing two responses would otherwise see a
/// difference between the implementations that no feature asked for.
pub async fn assign_request_id(mut request: Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/api/") {
        return next.run(request).await;
    }

    let id = chosen_id(request.headers());
    request.extensions_mut().insert(RequestId(id.clone()));

    let mut response = next.run(request).await;
    // `after_request` sets it unconditionally, so a handler that set its own —
    // `api_error_response` does — is overwritten with the same value.
    if let Ok(value) = axum::http::HeaderValue::from_str(&id) {
        response.headers_mut().insert(request_id_header(), value);
    }
    // A header value that cannot be built means the incoming id held bytes a
    // header cannot carry. Rather than answer 500 for a diagnostic field, the
    // response goes out without it; the generated-id path is hex and always
    // fits.
    response
}
