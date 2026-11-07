//! The HTTP half of `discover_lm_studio_models` (`agent.py:1529`).
//!
//! The source's discovery function is one flow that does two unrelated jobs: it asks an engine what
//! it is holding, and it decides what the app believes about the machine when the engine does not
//! say. The layer map (`PLAN-Rust-Clone.md` §3.3) splits those, and this file is the asking half plus the call
//! that hands the answer over: `GET {LM_ENGINE_HOST}/v1/models` → `parse_json_response_strict` →
//! `shed_models::apply_discovered_payload`, which owns the candidate parsing, the catalog, the cache
//! file and the disk scan. `LAYERS` (`layers.rs:36-45`) allows `shed-llm` to reach `shed-models` and
//! never the reverse, which is the whole reason the split runs this way rather than the other.
//!
//! ## The three ways one endpoint gives up
//!
//! A request that failed (1550-1552), a response whose status was not 200 (1554-1556), and a body
//! that was not JSON (1558-1562) all end in `continue`, which with one endpoint in `endpoints`
//! (1537) means the catalog is applied with nothing in hand and falls through to its own fallback
//! chain. So this file returns `Option<serde_json::Value>` and the catalog half never learns which
//! of the three happened — each one has already said so in the debug line the source wrote before
//! continuing.
//!
//! ## The `serde_json::Value` that passes through
//!
//! `PLAN-Rust-Clone.md` §3.3 bans a dynamic JSON value outside `shed-json` and the MCP argument
//! layer, and this is the corridor rather than the exception: the value is produced by `shed-json`,
//! handed straight to `shed-models`, and never stored or re-read here. The exception itself is
//! written in `shed-models/src/lib.rs`, where the value is consumed.

use std::path::Path;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::Value;
use tracing::{debug, info};

use shed_json::parse_json_response_strict;
use shed_models::{apply_discovered_payload, ModelCatalog};

use crate::retry::LmTransport;

/// `agent.py:1537`'s one endpoint, spelled the way the source spells it: the host already has its
/// trailing slashes removed (`393`), so the path is appended rather than joined.
const MODELS_PATH: &str = "/v1/models";

/// The name `agent.py:1559` hands to `parse_json_response_strict`, which is the name that then
/// appears in the message the parse-failure line at 1561 quotes:
/// `LM Studio model discovery returned non-JSON content (status=200, …)`.
const SOURCE_NAME: &str = "LM Studio model discovery";

/// The purpose the ladder prints (`agent.py:1549`), and the phrase the failure it raises is built
/// from: `LM request failed during model discovery: …`.
const PURPOSE: &str = "model discovery";

/// The URL this pass asks for: `LM_ENGINE_HOST` plus the models path, built in one place so the
/// line at 1535, the lines that name a URL inside the catalog half, and the request itself all say
/// the same address.
fn models_url(transport: &LmTransport) -> String {
    format!("{}{MODELS_PATH}", transport.host())
}

/// `discover_lm_studio_models`: ask the engine, apply the answer, and return the model names the
/// app now believes it has.
///
/// The return value is `KNOWN_MODELS` after the call (1607, 1612, 1618, 1625, 1629 — every branch
/// returns `list(KNOWN_MODELS)`), which is what `/api/refresh-models` (11601) puts in its JSON. At
/// boot the source discards it; the clone logs it, because a boot that says nothing about what it
/// found is a boot the operator cannot check.
///
/// `storage_root` and `models_root` are the two directories the fallback chain reads: the cache file
/// lives under the first, the disk scan walks the second, and neither is a path this crate decides —
/// they come from `AppConfig`. `models_root` is `None` when neither `LMSTUDIO_MODELS_DIR` nor
/// `USERPROFILE` names one, which is `agent.py:1322-1323`'s `None` and means the last fallback has
/// nothing to walk.
pub async fn discover_lm_studio_models(
    transport: &LmTransport,
    catalog: &mut ModelCatalog,
    storage_root: &Path,
    models_root: Option<&Path>,
    request_id: &str,
) -> Vec<String> {
    let payload = fetch_discovery_payload(transport, request_id).await;
    let url = models_url(transport);
    apply_discovered_payload(catalog, payload.as_ref(), &url, storage_root, models_root)
}

/// `agent.py:1535` and the body of the endpoint loop (1548-1562): ask the engine, and hand back what
/// it answered.
///
/// `AI_MODEL_DISCOVERY_TIMEOUT` is the timeout this call names, and it is a socket timeout in the
/// source rather than a budget for the whole exchange — `docs/PARITY.md`, "Structural changes".
async fn fetch_discovery_payload(transport: &LmTransport, request_id: &str) -> Option<Value> {
    let url = models_url(transport);
    info!("Discovering models via {url}...");

    let timeout = Duration::from_secs(transport.discovery_timeout_seconds());
    let response = match transport
        .call(request_id, Method::GET, &url, timeout, PURPOSE)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            // 1551. The ladder's own warning lines came first, if there were any to write.
            debug!("[{request_id}] Model discovery: request to {url} failed: {error}");
            return None;
        }
    };

    let status = response.status();
    if status != StatusCode::OK {
        // 1555. A status the ladder did not consider transient — a 401, a 404, a 500 — gets here,
        // because `call_lm_http_with_retry` returns a response whatever its status is (1068).
        debug!("Model discovery: {url} returned status {}", status.as_u16());
        return None;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    // `response.text` sits inside its own `try` at `utils/json_util.py:47-50`, where a body that
    // cannot be read becomes an empty string and the strict parse carries on to complain about that.
    let body = response.text().await.unwrap_or_default();

    match parse_json_response_strict(SOURCE_NAME, status.as_u16(), &content_type, &body) {
        Ok(payload) => Some(payload),
        Err(error) => {
            debug!("[{request_id}] Model discovery: {url} parse failure: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shed_config::{EnvSource, LmEngine};

    use super::*;
    use crate::retry::LmTransport;

    /// A transport pointed at a host written the way an operator writes it in `.env`.
    fn transport(host: &str) -> LmTransport {
        let engine = LmEngine::from_source(&EnvSource::from_file_values(BTreeMap::from([(
            "LM_ENGINE_HOST".to_owned(),
            host.to_owned(),
        )])));
        LmTransport::new(&engine)
    }

    #[test]
    fn the_url_is_the_configured_host_with_the_models_path_appended() {
        assert_eq!(
            models_url(&transport("http://127.0.0.1:8080")),
            "http://127.0.0.1:8080/v1/models"
        );
        // `agent.py:393` rstrips `/`, so a host written with them still yields one separator.
        assert_eq!(
            models_url(&transport("http://127.0.0.1:8080//")),
            "http://127.0.0.1:8080/v1/models"
        );
    }
}
