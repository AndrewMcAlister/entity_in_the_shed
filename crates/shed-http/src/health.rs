//! `GET /health` — the clone of `healthcheck` (`agent.py:11502–11516`).
//!
//! Byte contract captured from the running Python app (plan §11 U2): compact
//! JSON, keys sorted alphabetically (Flask `jsonify` sets `sort_keys`), a
//! trailing newline, `Content-Type: application/json` with no charset. A
//! `serde_json::Map` is a `BTreeMap` without the `preserve_order` feature,
//! so serialising reproduces Flask's ordering for free.
//!
//! Stage 1 divergence, recorded in PARITY.md: Python's `error` field prefers
//! the live MCP cache's last connection failure and falls back to the
//! validation message; with no Stage 5 client yet there is no cache, so the
//! clone reports the validation message or `""`. `tool_count`/`tools` are the
//! empty values Python shows when the server is unreachable.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Response};
use serde_json::{json, Value};
use shed_config::AppConfig;

pub type AppState = Arc<AppConfig>;

/// The handler. Flask also answers HEAD on a GET route; the browser never
/// sends one, so the clone keeps plain GET (405 otherwise, matching Flask's
/// method-not-allowed for POST).
pub async fn healthcheck(State(state): State<AppState>) -> Response<Body> {
    let mut response = Response::new(Body::from(payload_text(&state)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// The full body bytes: compact JSON plus the newline `jsonify` appends.
#[must_use]
pub fn payload_text(config: &AppConfig) -> String {
    // Value's Display is the compact serialisation; it cannot fail, unlike
    // to_string's Result, which nothing downstream could act on anyway.
    let mut text = payload(config).to_string();
    text.push('\n');
    text
}

/// The payload as a value; insertion order is irrelevant because the map
/// serialises sorted.
#[must_use]
pub fn payload(config: &AppConfig) -> Value {
    json!({
        "status": "ok",
        "AI_MODEL": config.ai_model,
        "lm_engine_normal_model": config.lm_engine_normal_model,
        "mctrader_mcp": {
            "configured": config.mcp.validate().is_ok(),
            "transport": config.mcp.transport(),
            "server_name": config.mcp.server_name,
            "tool_count": 0,
            "tools": [],
            "error": config.mcp.health_error(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::payload_text;
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, HeaderValue, Request, StatusCode};
    use http_body_util::BodyExt;
    use shed_config::{AppConfig, EnvSource};
    use tower::ServiceExt;

    /// Build a config through the real parser; the storage root points at a
    /// temp directory so the test never touches the default one.
    fn config(pairs: &[(&str, &str)]) -> Arc<AppConfig> {
        let root = std::env::temp_dir().join(format!("shed-http-health-{}", std::process::id()));
        let root_text = root.to_string_lossy().into_owned();
        let mut owned: Vec<(&str, &str)> = pairs.to_vec();
        owned.push(("STORAGE_ROOT", root_text.as_str()));
        let source = EnvSource::from_file_values(
            owned
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        );
        match AppConfig::from_source(&source, Path::new(".")) {
            Ok(config) => Arc::new(config),
            Err(error) => panic!("synthetic config must parse: {error}"),
        }
    }

    /// Byte parity with a live capture's shape: sorted keys, compact
    /// separators, trailing newline. The capture's own `error` read
    /// "All connection attempts failed" (the Python MCP cache, Stage 5);
    /// a configured-and-valid clone config reports "".
    #[test]
    fn body_bytes_match_the_flask_shape() {
        let text = payload_text(&config(&[
            ("AI_MODEL", ""),
            ("LM_ENGINE_NORMAL_MODEL", "Some Model (Q4_K_M)"),
            ("MCTRADER_MCP_TRANSPORT", "streamable-http"),
            ("MCTRADER_MCP_URL", "http://127.0.0.1:8100/mcp"),
        ]));
        assert_eq!(
            text,
            concat!(
                r#"{"AI_MODEL":"","lm_engine_normal_model":"Some Model (Q4_K_M)","#,
                r#""mctrader_mcp":{"configured":true,"error":"","server_name":"mctrader","#,
                r#""tool_count":0,"tools":[],"transport":"streamable-http"},"status":"ok"}"#,
                "\n"
            )
        );
    }

    #[test]
    fn unconfigured_reports_the_validation_message() {
        let text = payload_text(&config(&[]));
        assert!(text.contains(r#""configured":false"#), "{text}");
        assert!(
            text.contains(r#"Set MCTRADER_MCP_COMMAND, MCTRADER_MCP_URL, or set MCTRADER_MCP_TRANSPORT explicitly."#),
            "{text}"
        );
        assert!(text.ends_with("}\n"), "{text}");
    }

    async fn body_of(response: axum::response::Response) -> String {
        let collected = match response.into_body().collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(error) => panic!("body collection failed: {error}"),
        };
        match String::from_utf8(collected.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("body not utf-8: {error}"),
        }
    }

    /// `Request::builder().body()` yields a `Result`; the lints deny unwrap,
    /// so the test seams turn the (impossible) builder error into a panic.
    fn request(builder: axum::http::request::Builder) -> Request<Body> {
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    #[tokio::test]
    async fn get_health_serves_the_contract_headers() {
        let state = config(&[("MCTRADER_MCP_URL", "http://127.0.0.1:8100/mcp")]);
        let expected = payload_text(&state);
        let response = match crate::build_router(state)
            .oneshot(request(Request::get("/health")))
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("oneshot failed: {error}"),
        };
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(HeaderValue::as_bytes),
            Some(b"application/json".as_slice()),
            "Flask sends application/json with no charset"
        );
        assert_eq!(body_of(response).await, expected);
    }

    #[tokio::test]
    async fn other_requests_get_flask_like_statuses() {
        let state = config(&[]);
        let response = match crate::build_router(state)
            .oneshot(request(Request::post("/health")))
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("oneshot failed: {error}"),
        };
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
