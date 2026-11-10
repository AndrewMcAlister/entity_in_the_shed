//! `GET /` — the page, `index` (`agent.py:8418-8430`).
//!
//! Jinja's job here is three values. `templates/index.html` is a frozen asset
//! and it contains exactly three template expressions, at lines 132, 678 and
//! 679:
//!
//! ```text
//! const modelSelectionRequiresPassword = {{ model_selection_requires_password|tojson }};
//! const chatRequestTimeoutMs = {{ chat_request_timeout_ms|tojson }};
//! const browserTtsEnabled = {{ browser_tts_enabled|tojson }};
//! ```
//!
//! So that is what this does: reads the file once and replaces those three
//! literal strings with what `|tojson` would have written for the settings the
//! server booted with. A full Jinja would be able to do more, and doing more
//! is the reason not to — the page is not a template a person edits per
//! request, it is one file with three holes in it, and a reader can see all
//! three holes here.
//!
//! The `|tojson` filter is Flask's `htmlsafe_dumps`: JSON with `<`, `>`, `&`,
//! `'` and `"` escaped as `\u003c` and friends. All three values are booleans
//! or integers, which that escaping leaves untouched, so `true`/`false`/`75000`
//! is exactly what Flask writes. That equivalence is why the substitution is
//! literal text rather than a JSON encoder pointed at a template AST: no value
//! this page can be handed will ever need the escaping.
//!
//! A missing expression is an error, not a no-op. The page's inline script is
//! one `<script>` block, so an unsubstituted `{{ ... }}` is a syntax error and
//! the whole page — including the send button — stops working. Failing the
//! request instead says "the shipped template is not the one this server was
//! built for", which is the truth, and says it in the log rather than in a
//! blank browser tab.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{self, CONTENT_TYPE, PRAGMA};
use axum::http::{HeaderValue, Response, StatusCode};

use crate::state::AppState;

/// The page asks for its stylesheet and helpers by these names, so the
/// substitution keys and the asset routes below are the same strings the
/// browser sends. Nothing else may rename them.
const PAGE_TOKENS: [(&str, Token); 3] = [
    (
        "{{ model_selection_requires_password|tojson }}",
        Token::PasswordGate,
    ),
    ("{{ chat_request_timeout_ms|tojson }}", Token::ReplyTimeout),
    ("{{ browser_tts_enabled|tojson }}", Token::BrowserTts),
];

/// Which of the three holes a literal expression is.
#[derive(Clone, Copy)]
enum Token {
    /// `MODEL_SELECTION_REQUIRES_PASSWORD` (`agent.py:8424`).
    PasswordGate,
    /// `chat_request_timeout_ms`, computed at `agent.py:8420`.
    ReplyTimeout,
    /// `bool(TTS_BROWSER_FALLBACK)` (`agent.py:8425`).
    BrowserTts,
}

/// The four files the browser names in a URL, resolved against `BASE_DIR`.
///
/// `STYLE_DIR`/`STATIC_DIR` (`agent.py:302-303`) are the source's own names for
/// two of these; the other two are what `render_template("index.html")` and the
/// `styles/` route resolve.
pub struct PagePaths {
    /// `templates/index.html`, what `index` renders.
    pub index: PathBuf,
    /// `STYLE_DIR / "app.css"`, served at `/app.css`.
    pub stylesheet: PathBuf,
    /// `STATIC_DIR / "app-helpers.js"`, served at `/app-helpers.js`.
    pub helpers: PathBuf,
    /// `styles/information_info_1565.webp`, served at the same path.
    pub info_icon: PathBuf,
}

impl PagePaths {
    /// Resolve the four paths against the base directory.
    #[must_use]
    pub fn new(base_dir: &Path) -> Self {
        Self {
            index: base_dir.join("templates").join("index.html"),
            stylesheet: base_dir.join("styles").join("app.css"),
            helpers: base_dir.join("static").join("app-helpers.js"),
            info_icon: base_dir.join("styles").join("information_info_1565.webp"),
        }
    }
}

/// The handler.
pub async fn index(State(state): State<AppState>) -> Response<Body> {
    let page = match state.cached_page() {
        Some(page) => page.to_vec(),
        None => match render_page(&state) {
            Ok(rendered) => state.store_page(rendered).to_vec(),
            // The source has no `try` here, and the failure it would have had
            // to survive — a template file that is missing, or present but not
            // the one this server was built for — is a broken install rather
            // than a request problem. Flask answers that with werkzeug's own
            // 500 page; the clone answers with one line of text and logs the
            // reason, because a browser pointed at this URL is a person
            // debugging a deployment, not a person chatting.
            Err(error) => {
                tracing::error!("page render failed: {error}");
                return text_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    b"internal server error\n".to_vec(),
                );
            }
        },
    };

    let mut response = Response::new(Body::from(page));
    let headers = response.headers_mut();
    no_store(headers);
    // Flask adds the charset itself (`werkzeug`'s default for a text response,
    // measured against the running app), so the clone writes the same bytes
    // rather than the shorter `text/html` it would have chosen on its own.
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// The three cache headers, written once here and by every asset route.
///
/// All three, not just `Cache-Control` (`agent.py:8427-8429`, `8440-8442`,
/// `8463-8465`, `8476-8478`): the page and the code it loads must never come
/// from a cache, because a page one deploy old asks for helpers that no longer
/// exist, and the browser then throws "not a function" and loses the reply.
pub(super) const NO_STORE: &str = "no-store, no-cache, max-age=0, must-revalidate";

/// Add the three cache headers to a response built elsewhere.
pub(super) fn no_store(headers: &mut axum::http::HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_STORE));
    headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::EXPIRES, HeaderValue::from_static("0"));
}

/// `make_response(render_template("index.html", …))` for the one template this
/// app renders.
fn render_page(state: &AppState) -> Result<Vec<u8>, String> {
    let path = &state.paths.index;
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text = jinja_source(&raw);

    let mut rendered = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while !rest.is_empty() {
        // Take the earliest of the three holes, so the text between them is
        // copied through untouched and a hole that appears twice in the file
        // (none does) would still be filled.
        let next = PAGE_TOKENS
            .iter()
            .filter_map(|(literal, kind)| rest.find(literal).map(|at| (at, *literal, *kind)))
            .min_by_key(|(at, _, _)| *at);
        match next {
            Some((at, literal, kind)) => {
                rendered.push_str(&rest[..at]);
                rendered.push_str(&substitution(kind, state));
                rest = &rest[at + literal.len()..];
            }
            None => {
                rendered.push_str(rest);
                break;
            }
        }
    }

    // Every hole must have been present: a template that lost one of them is
    // a template this server was not built for, and the value it carried
    // would silently never reach the page.
    for (literal, _) in &PAGE_TOKENS {
        if !text.contains(literal) {
            return Err(format!("{} is missing from {}", literal, path.display()));
        }
    }
    Ok(rendered.into_bytes())
}

/// The two text-mode habits of Jinja that change the page's bytes.
///
/// Jinja's loader reads a template through `open(..., newline=None)` —
/// Python's universal newlines, so a CRLF checkout arrives as LF — and its
/// lexer then drops exactly one trailing newline (`keep_trailing_newline`,
/// `False` by default, applied to the normalised source).
/// `templates/index.html` is a CRLF file ending `</html>\r\n` in *both* apps'
/// working copies, byte-identical files 193592 of them, so without these two
/// steps the clone ships 4531 bytes the source does not: 4530 line endings
/// plus the last newline. Measured against the running app on that one file —
/// with both steps the page is byte-identical, without them it is not.
///
/// Only the page goes through this. The stylesheet, the helpers and the icon
/// reach the browser through `send_file`, which streams raw bytes and leaves
/// the CRLF alone, and the clone's asset routes match those three byte for
/// byte as they stand.
fn jinja_source(text: &str) -> String {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    match normalised.strip_suffix('\n') {
        Some(stripped) => stripped.to_owned(),
        None => normalised,
    }
}

/// What `|tojson` writes for one of the three values.
fn substitution(token: Token, state: &AppState) -> String {
    match token {
        Token::PasswordGate => py_bool(state.config.model_selection_requires_password),
        // `None if not LM_ENGINE_REPLY_TIMEOUT else LM_ENGINE_REPLY_TIMEOUT * 1000`
        // (`agent.py:8420`): no timer at all is `null`, which is what stops the
        // page from aborting a request the server has decided to keep waiting
        // for. Seconds in, milliseconds out, because `fetch` takes milliseconds.
        Token::ReplyTimeout => match state.config.lm.reply_timeout_seconds {
            Some(seconds) if seconds != 0 => (seconds * 1000).to_string(),
            _ => "null".to_owned(),
        },
        Token::BrowserTts => py_bool(state.config.tts.browser_fallback),
    }
}

/// Jinja's rendering of a Python `bool`, which is also its rendering of the
/// `0`/`1` the settings parser produces.
fn py_bool(value: bool) -> String {
    if value {
        "true".to_owned()
    } else {
        "false".to_owned()
    }
}

/// A `text/plain` response — the shape of the two text bodies the
/// last-model-index routes serve (`agent.py:8709`, `agent.py:8713`).
///
/// No cache headers, because the source sends none here: its `make_response`
/// is not a `send_file`, so Flask leaves `Cache-Control` off the response
/// entirely (measured against the running app). The four no-store routes are
/// the contract; inventing a fifth header set for a body the Python app ships
/// bare would be a response no capture of the source matches.
pub(super) fn text_response(status: StatusCode, body: Vec<u8>) -> Response<Body> {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}
