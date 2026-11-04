//! `GET /health`: the settings the app is answering with.
//!
//! Ported from `agent.py:11502` (`healthcheck`) and `agent.py:1936` (`get_mctrader_mcp_status`).
//!
//! The payload is built as a `serde_json::Map`, which orders its keys by code point, which is
//! what `jsonify` does: the source writes its dict as `status, AI_MODEL, lm_engine_normal_model,
//! mctrader_mcp` and the response still reads `AI_MODEL, lm_engine_normal_model, mctrader_mcp,
//! status`. Order is part of the bytes a captured response is compared against, so it is not
//! incidental.

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use shed_config::McpConfig;

use crate::AppState;

/// What Flask writes, and what `axum::Json` writes: no `charset`, because JSON is UTF-8 by
/// definition and the source never named one.
const JSON_CONTENT_TYPE: &str = "application/json";

/// `jsonify` ends the body with a newline, and `Content-Length` counts it. It is a byte of the
/// response rather than decoration, so it is written here rather than left to a client library.
const TRAILING_NEWLINE: &str = "\n";

/// `GET /health`.
pub async fn handler(State(state): State<AppState>) -> Response {
    let config = &state.config;

    let mut payload = Map::new();
    payload.insert("status".to_string(), Value::from("ok"));
    payload.insert("AI_MODEL".to_string(), Value::from(config.models.ai_model.as_str()));
    payload.insert(
        "lm_engine_normal_model".to_string(),
        Value::from(config.models.normal_model.as_str()),
    );
    payload.insert("mctrader_mcp".to_string(), mcp_status(&config.mcp));

    // Writing this payload cannot fail: it holds no number that has no JSON spelling, and no
    // reader or writer is involved. `unreachable!` is how this workspace states that without an
    // `expect`, which the workspace denies.
    let body = serde_json::to_string(&payload)
        .unwrap_or_else(|err| unreachable!("the health payload cannot be written: {err}"));

    json_response(StatusCode::OK, body)
}

/// `get_mctrader_mcp_status`, in the shape `/health` reports it.
///
/// `tool_count` and `tools` are the empty answer because Stage 1 does not open a session with
/// the server; the MCP client that fills them arrives in Stage 5. Everything else here is the
/// real setting, read and validated the way the source reads and validates it.
fn mcp_status(mcp: &McpConfig) -> Value {
    let mut status = Map::new();
    status.insert("configured".to_string(), Value::from(mcp.is_configured()));
    status.insert("transport".to_string(), Value::from(mcp.transport_text()));
    status.insert("server_name".to_string(), Value::from(mcp.server_name.as_str()));
    status.insert("tool_count".to_string(), Value::from(0));
    status.insert("tools".to_string(), Value::Array(Vec::new()));
    // `cache_error or error_message`: a configured server reports no error at all, which is an
    // empty string rather than an absent key.
    status.insert("error".to_string(), Value::from(mcp.validation_error().unwrap_or_default()));
    Value::Object(status)
}

/// The response every JSON route writes: a status, one content type, and a body that is already
/// JSON text.
///
/// `axum::Json` is not used because it writes the body without the newline `jsonify` writes.
fn json_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE))],
        body + TRAILING_NEWLINE,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::error::Error;
    use std::path::PathBuf;

    use axum::body::{Body, to_bytes};
    use axum::http::{HeaderMap, Method, Request};
    use shed_config::{AppConfig, ConfigError, Env};
    use tower::ServiceExt;

    use crate::{AppState, router};

    /// The body `GET /health` answered in the Python app running this operator's own `.env`,
    /// captured off the live server: 200, `Content-Type: application/json`,
    /// `Content-Length: 289`, and no `X-Request-ID` because `/health` is not an `/api/` route.
    const CAPTURED_BODY: &str = concat!(
        r#"{"AI_MODEL":"","lm_engine_normal_model":"Qwen3-Coder-30B-A3B-Instruct-Q6_K","mctrader_mcp":{"configured":false,"error":"Set MCTRADER_MCP_COMMAND, MCTRADER_MCP_URL, or set MCTRADER_MCP_TRANSPORT explicitly.","server_name":"mctrader","tool_count":0,"tools":[],"transport":""},"status":"ok"}"#,
        "\n",
    );

    /// A directory of its own under the approved temporary tree, so `STORAGE_ROOT` has somewhere
    /// to be created and nothing the app owns is written inside the repo.
    fn scratch(label: &str) -> Result<PathBuf, ConfigError> {
        let dir = std::env::temp_dir().join("shed-http-health-tests").join(label);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|source| ConfigError::Storage { path: dir.display().to_string(), source })?;
        Ok(dir)
    }

    fn config(label: &str, pairs: &[(&str, &str)]) -> Result<AppConfig, ConfigError> {
        let env = Env::with_values(
            pairs.iter().map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        );
        AppConfig::load_from(&env, &scratch(label)?)
    }

    /// Calls a route in memory: no socket, no port, the same router the server is handed.
    async fn call(
        method: Method,
        uri: &str,
        config: AppConfig,
    ) -> Result<(StatusCode, HeaderMap, String), Box<dyn Error + Send + Sync>> {
        let request = Request::builder().method(method).uri(uri).body(Body::empty())?;
        let response = router(AppState::new(config)).oneshot(request).await?;
        let (parts, body) = response.into_parts();
        let bytes = to_bytes(body, usize::MAX).await?;
        Ok((parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// The whole point of the fixture: the same settings produce the same bytes.
    #[tokio::test]
    async fn the_health_route_answers_with_the_captured_body()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let config =
            config("captured", &[("LM_ENGINE_NORMAL_MODEL", "Qwen3-Coder-30B-A3B-Instruct-Q6_K")])?;
        let (status, headers, body) = call(Method::GET, "/health", config).await?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, CAPTURED_BODY);
        assert_eq!(body.len(), 289, "the captured Content-Length");
        assert_eq!(
            headers.get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );
        // `/health` is not under `/api/`, so the request-id header the API routes carry is not
        // written here.
        assert!(headers.get("x-request-id").is_none());
        Ok(())
    }

    /// A configured server is reported as configured, with no error text at all. `tool_count`
    /// and `tools` are the Stage 1 answer: nothing is connected yet.
    #[tokio::test]
    async fn a_configured_stdio_server_is_reported_configured()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config("stdio", &[("MCTRADER_MCP_COMMAND", "cmd.exe")])?;
        let (_, _, body) = call(Method::GET, "/health", config).await?;
        let payload: Value = serde_json::from_str(&body)?;
        let mcp = &payload["mctrader_mcp"];

        assert_eq!(mcp["configured"], Value::from(true));
        assert_eq!(mcp["transport"], Value::from("stdio"));
        assert_eq!(mcp["server_name"], Value::from("mctrader"));
        assert_eq!(mcp["error"], Value::from(""));
        assert_eq!(mcp["tool_count"], Value::from(0));
        assert_eq!(mcp["tools"], Value::Array(Vec::new()));
        Ok(())
    }

    /// A transport that is named but cannot be reached is reported as not configured, with the
    /// reason the source gives.
    #[tokio::test]
    async fn a_transport_without_the_setting_that_reaches_it_is_reported_broken()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config("broken", &[("MCTRADER_MCP_TRANSPORT", "streamable-http")])?;
        let (status, _, body) = call(Method::GET, "/health", config).await?;
        let payload: Value = serde_json::from_str(&body)?;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["status"], Value::from("ok"));
        assert_eq!(payload["mctrader_mcp"]["configured"], Value::from(false));
        assert_eq!(
            payload["mctrader_mcp"]["error"],
            Value::from("MCTRADER_MCP_URL is not configured")
        );
        Ok(())
    }

    /// The route answers GET and nothing else, which is what `methods=["GET"]` means in the
    /// source. Axum answers the same way without a handler of its own.
    #[tokio::test]
    async fn a_post_to_health_is_not_allowed() -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config("post", &[])?;
        let (status, _, _) = call(Method::POST, "/health", config).await?;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        Ok(())
    }

    /// A path with no route is the 404 Flask answers, not an error page.
    #[tokio::test]
    async fn a_path_this_stage_does_not_answer_is_not_found()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let config = config("missing", &[])?;
        let (status, _, _) = call(Method::GET, "/api/chat", config).await?;
        assert_eq!(status, StatusCode::NOT_FOUND);
        Ok(())
    }
}
