//! `tests/test_frontend_cache_headers.py` — the page and the two code files it
//! loads must arrive with the no-store trio.
//!
//! The Python suite drives `agent.app.test_client()` and patches module
//! globals to move a setting; here the router is oneshotted over a state built
//! from the settings the page reads, which is the same lever one level up
//! (`AppConfig` is what the source's globals became). The base directory is
//! the repository root, so the page and the assets are the real files the
//! routes serve.
//!
//! Three Python cases are not ported here. `test_page_uses_no_store_for_audio_fetches`
//! is `pass` in the source itself — nothing to port. The `/api/speak`,
//! `/api/reply-laugh` and `/api/reply-fart` cases arrive with the voice
//! stage, those routes not existing yet. What this file pins is the four
//! files the page is built from, including the 404 the source's `send_file`
//! raises when one of them is gone.

//! The test binary links these crates through the library but names none of
//! them itself; the workspace's `unused_crate_dependencies` lint judges the
//! binary on its own uses.
use serde as _;
use serde_json as _;
use shed_characters as _;
use shed_llm as _;
use shed_text as _;
use shed_tts as _;
use tracing as _;
use uuid as _;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, EXPIRES, PRAGMA};
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use shed_config::{AppConfig, EnvSource};
use shed_http::build_router;
use shed_http::state::{AppCore, AppState};
use shed_models::ModelCatalog;
use tower::ServiceExt;

/// The repository root — the clone's `BASE_DIR`, where `templates/`,
/// `styles/` and `static/` live next to the crates.
fn base_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Build a config through the real parser over a chosen base directory; the
/// storage root points at a temp directory so the test never touches the
/// default one.
fn config(pairs: &[(&str, &str)], root: &Path) -> AppConfig {
    let storage =
        std::env::temp_dir().join(format!("shed-http-cache-headers-{}", std::process::id()));
    let storage_text = storage.to_string_lossy().into_owned();
    let mut owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    owned.push(("STORAGE_ROOT".to_owned(), storage_text));
    let source = EnvSource::from_file_values(owned.into_iter().collect());
    match AppConfig::from_source(&source, root) {
        Ok(parsed) => parsed,
        Err(error) => panic!("synthetic config must parse: {error}"),
    }
}

/// The router's state for one set of settings. The catalog is the empty one:
/// none of these routes reads it, and each state gets its own `OnceLock`, so
/// a page rendered for one setting never leaks into the next test.
fn state(pairs: &[(&str, &str)], root: &Path) -> AppState {
    Arc::new(AppCore::new(config(pairs, root), ModelCatalog::default()))
}

/// One GET through the real router, with the body already collected.
async fn get(state: AppState, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = match Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
    {
        Ok(request) => request,
        Err(error) => panic!("synthetic request: {error}"),
    };
    let response = match build_router(state).oneshot(request).await {
        Ok(response) => response,
        Err(error) => panic!("router failure: {error}"),
    };
    let (parts, body) = response.into_parts();
    let collected = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => panic!("body collection failed: {error}"),
    };
    (parts.status, parts.headers, collected.to_vec())
}

/// The trio, asserted the way the source's four routes ship it. A missing
/// header is as much a failure as a wrong value: a browser that sees no
/// `Cache-Control` is free to serve the page from six deploys ago.
fn assert_no_store(headers: &HeaderMap) {
    for (name, value) in [
        (
            CACHE_CONTROL,
            "no-store, no-cache, max-age=0, must-revalidate",
        ),
        (PRAGMA, "no-cache"),
        (EXPIRES, "0"),
    ] {
        let found = headers
            .get(name.clone())
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let actual = found.unwrap_or_else(|| panic!("{} missing from the response", name.as_str()));
        assert_eq!(actual, value, "{}", name.as_str());
    }
}

fn text(bytes: &[u8]) -> String {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => text,
        Err(error) => panic!("body not utf-8: {error}"),
    }
}

#[tokio::test]
async fn page_sets_no_cache_headers() {
    let (status, headers, body) = get(
        state(&[("LM_ENGINE_REPLY_TIMEOUT", "75")], &base_dir()),
        "/",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_no_store(&headers);
    assert!(
        text(&body).contains("const chatRequestTimeoutMs = 75000;"),
        "seconds in, milliseconds out — the page's fetch timer"
    );
}

#[tokio::test]
async fn page_can_disable_chat_request_timeout() {
    let (status, _, body) = get(
        state(&[("LM_ENGINE_REPLY_TIMEOUT", "none")], &base_dir()),
        "/",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text(&body).contains("const chatRequestTimeoutMs = null;"),
        "no timer at all is null, which stops the page aborting a reply the server chose to keep waiting for"
    );
}

#[tokio::test]
async fn page_browser_tts_flag_disabled() {
    let (status, _, body) = get(state(&[("TTS_BROWSER_FALLBACK", "0")], &base_dir()), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text(&body).contains("const browserTtsEnabled = false;"),
        "0 is server voices only, and Jinja renders the setting as a bool"
    );
}

#[tokio::test]
async fn page_injects_browser_tts_enabled() {
    let (status, _, body) = get(state(&[("TTS_BROWSER_FALLBACK", "1")], &base_dir()), "/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(text(&body).contains("const browserTtsEnabled = true;"));
}

#[tokio::test]
async fn app_stylesheet_sets_no_cache_headers() {
    let (status, headers, body) = get(state(&[], &base_dir()), "/app.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_no_store(&headers);
    assert!(
        text(&body).contains(".shell"),
        "the real file, and the rule whose silent deletion this whole suite is about"
    );
}

#[tokio::test]
async fn page_loads_helpers_from_the_no_store_route() {
    // The page is no-store, so it is always newer than anything a cache
    // holds. If it loaded the helpers from Flask's static route, a browser
    // or tunnel holding an older copy would answer a helper the fresh page
    // expects with "not a function" — which cost a reply for real.
    let (_, _, body) = get(state(&[], &base_dir()), "/").await;
    let page = text(&body);
    assert!(page.contains(r#"<script src="/app-helpers.js"></script>"#));
    assert!(!page.contains("static/app-helpers.js"));
}

#[tokio::test]
async fn page_has_no_preconfigured_phrase_path() {
    // The entity used to speak a canned line ("Let me think about that for
    // a moment.") before every answer. The page's inline script is not
    // loadable by the JS suite harness, so the removal is pinned here: a
    // status phrase can only come back by way of this page.
    let (status, _, body) = get(state(&[], &base_dir()), "/").await;
    assert_eq!(status, StatusCode::OK);
    let page = text(&body);
    assert!(!page.contains("status_phrase"));
    assert!(!page.contains("speakStatusPhrase"));
}

#[tokio::test]
async fn page_resumes_the_country_but_never_the_model() {
    // The helper suite proves the decision; this pins that the page actually
    // asks for it. The inline script is not loadable by the JS harness, so
    // without this the wiring could be deleted and every suite would pass.
    let (_, _, body) = get(state(&[], &base_dir()), "/").await;
    let page = text(&body);
    assert!(page.contains("chooseRestoredCountry"));
    assert!(page.contains("persistSelectedCountry"));
    // And the model stays the server's to choose: resuming it browser-side
    // would pin a page to a model the server had since swapped out.
    assert!(!page.contains("entity-selected-model"));
}

#[tokio::test]
async fn page_serves_lf_lines_and_no_trailing_newline() {
    // Jinja reads its template in text mode — universal newlines, so a CRLF
    // checkout arrives as LF — and drops one trailing newline
    // (`keep_trailing_newline=False`). `templates/index.html` is a CRLF file
    // ending `</html>\r\n`, and the running Python app answers it that way:
    // 188941 bytes of LF with no last newline, from a 193592-byte file. The
    // clone served 193493 bytes until both habits were ported, which is 4531
    // bytes of page the source never sends — invisible to a browser, and the
    // first thing the Stage-9 byte sweep would have flagged.
    let state = state(&[], &base_dir());
    let (_, _, body) = get(state.clone(), "/").await;
    let page = text(&body);
    let on_disk = match std::fs::read(&state.paths.index) {
        Ok(bytes) => bytes,
        Err(error) => panic!("the template the route reads must be readable: {error}"),
    };
    let lf_on_disk = on_disk.iter().filter(|byte| **byte == b'\n').count();
    assert!(
        !page.contains('\r'),
        "the source's text-mode read leaves no CR in the page"
    );
    assert!(
        !page.ends_with('\n'),
        "the lexer's dropped trailing newline must not come back"
    );
    assert!(page.ends_with("</html>"), "and the page still ends");
    assert_eq!(
        page.matches('\n').count(),
        lf_on_disk - 1,
        "one line ending per line on disk, minus the one the lexer drops"
    );
}

#[tokio::test]
async fn app_helpers_set_no_cache_headers() {
    let state = state(&[], &base_dir());
    let (status, headers, body) = get(state.clone(), "/app-helpers.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_no_store(&headers);
    // The route must serve the real file, not an empty or truncated one.
    let helpers = text(&body);
    assert!(helpers.contains("readNewSegments"));
    let on_disk = match std::fs::read(state.paths.helpers.clone()) {
        Ok(bytes) => bytes,
        Err(error) => panic!("the helpers file the route reads must be readable: {error}"),
    };
    assert_eq!(body, on_disk, "the bytes served are the bytes on disk");
}

#[tokio::test]
async fn app_helpers_route_404s_without_the_file() {
    // The Python test patches STATIC_DIR to an empty temp directory; the
    // clone's lever is the base directory the four paths resolve against,
    // pointed at a directory that was never created — a missing file and an
    // absent directory are the same 404 to `std::fs::read`.
    let nowhere = std::env::temp_dir().join(format!(
        "shed-http-no-static-{}-{}",
        std::process::id(),
        line!()
    ));
    let (status, _, _) = get(state(&[], &nowhere), "/app-helpers.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
