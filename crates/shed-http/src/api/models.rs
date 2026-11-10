//! The model list, the last-picked-index pair, and the refresh
//! (`agent.py:8545-8562`, `8702-8742`, `11595-11603`).
//!
//! The three share more than the module: all of them read or write the one
//! catalog the process booted with, which is why the catalog lives in
//! [`AppCore`] behind a lock rather than in a local the handler builds.
//! `last_model_index.txt` is the same story one level down — the source
//! persists the picker's position in a file under `BASE_DIR` so a restart
//! reopens the app on the model it was using, and the clone writes the same
//! file so the two apps could hand the setting back and forth during the
//! migration.

use std::path::Path;

use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use shed_models::format_model_display_name;

use crate::api::{text_report_format, Args};
use crate::json::{internal_error, json_response, jsonify};
use crate::page::text_response;
use crate::request_id::RequestId;
use crate::state::AppState;

/// `GET /api/models` — `api_models` (`agent.py:8545-8562`).
///
/// The list is `KNOWN_MODELS` in order — the discovery sort put the
/// smallest model first and the picker opens on it — and each entry's
/// display text is the metadata display, falling back to the key's part
/// before `@`, then through `format_model_display_name` for the context
/// suffix. The two OpenAI entries are appended verbatim when a key is
/// configured; the source appends them unconditionally in that `if`, so
/// the clone does too, duplicates and all.
pub async fn models(State(state): State<AppState>) -> Response<Body> {
    let models = state.with_models(|catalog| {
        catalog
            .known_models()
            .iter()
            .map(|name| {
                let meta = catalog.metadata_for(name).or_else(|| {
                    catalog.metadata_for(name.split_once('@').map_or(name.as_str(), |stem| stem.0))
                });
                let display = match meta.map(|m| m.display.as_str()).filter(|d| !d.is_empty()) {
                    Some(display) => display.to_owned(),
                    None => name.clone(),
                };
                json!({
                    "key": name,
                    "display": format_model_display_name(
                        catalog,
                        Some(name),
                        &display,
                        &state.context,
                    ),
                })
            })
            .collect::<Vec<Value>>()
    });

    let mut models = models;
    if !state.config.openai_api_key.is_empty() {
        models.push(json!({"key": "gpt-4", "display": "GPT-4 (OpenAI)"}));
        models.push(json!({
            "key": "gpt-3.5-turbo",
            "display": "GPT-3.5 Turbo (OpenAI)",
        }));
    }
    jsonify(&json!({ "models": models }))
}

/// `GET /api/get-last-model-index` — `api_get_last_model_index`
/// (`agent.py:8702-8714`).
///
/// The text form answers the bare digits with no newline — `make_response`
/// writes exactly `str(last_index)` — and the not-found text form is an
/// empty 404 body, not the JSON error. The client asks for the text form
/// when a saved index feeds a shell script rather than the picker.
pub async fn last_model_index(
    State(state): State<AppState>,
    Query(args): Query<Args>,
) -> Response<Body> {
    let as_text = text_report_format(&args);
    match load_index(&state.config.base_dir.join("last_model_index.txt")) {
        Some(index) if as_text => text_response(StatusCode::OK, index.to_string().into_bytes()),
        Some(index) => jsonify(&json!({ "ok": true, "index": index })),
        None if as_text => text_response(StatusCode::NOT_FOUND, Vec::new()),
        None => json_response(
            &json!({"ok": false, "error": "Last model index not found"}),
            StatusCode::NOT_FOUND,
            None,
            None,
        ),
    }
}

/// `POST /api/set-last-model-index` — `api_set_last_model_index`
/// (`agent.py:8717-8725`).
///
/// `request.get_json()` with no arguments is the one Flask call on this
/// path that raises for a *request* problem — no body, a non-JSON
/// Content-Type, malformed JSON, or a JSON body that is not an object all
/// raise — and `api_json_boundary` turns every one of them into the 500
/// envelope, which is what [`internal_error`] writes here. A well-formed
/// `{"index": ...}` whose value is not a non-negative integer answers
/// `{"error": "Invalid index"}` at 400, the view's own last line.
pub async fn set_last_model_index(
    State(state): State<AppState>,
    id: RequestId,
    request: Request,
) -> Response<Body> {
    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(&bytes) else {
        return internal_error(
            "/api/set-last-model-index",
            "request.get_json() raised",
            &id.0,
        );
    };

    // `isinstance(index, int) and index >= 0`. A JSON float is not a Python
    // int and a JSON `true` technically is one — the source would then save
    // the text "True", which its own loader cannot read back. The clone
    // rejects both shapes at the same line rather than persisting a value
    // nothing can load.
    let index = body
        .get("index")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0);
    match index {
        Some(index) => {
            save_index(&state.config.base_dir.join("last_model_index.txt"), index);
            jsonify(&json!({ "status": "ok" }))
        }
        None => json_response(
            &json!({"error": "Invalid index"}),
            StatusCode::BAD_REQUEST,
            None,
            None,
        ),
    }
}

/// `POST /api/refresh-models` — `api_refresh_models`
/// (`agent.py:11595-11603`).
///
/// Discovery runs again and *its* result replaces the served catalog —
/// the source's `discover_lm_studio_models()` rewrites the `KNOWN_MODELS`
/// global in place and the route answers from it afterwards. The swap is
/// one lock and one move; the discovery itself, which is a network call and
/// a directory walk, happens into a fresh catalog so no lock is held across
/// the await. The source's `except` arm has no trigger here for the reason
/// the module comment in [`crate::api`] gives.
pub async fn refresh_models(State(state): State<AppState>, id: RequestId) -> Response<Body> {
    let transport = shed_llm::LmTransport::new(&state.config.lm);
    let mut fresh = shed_models::ModelCatalog::default();
    shed_llm::discover_lm_studio_models(
        &transport,
        &mut fresh,
        &state.config.storage_root,
        state.config.lm_studio.models_dir.as_deref(),
        &id.0,
    )
    .await;
    let known = fresh.known_models().to_vec();
    state.replace_models(fresh);
    jsonify(&json!({
        "ok": true,
        "known_models": known,
        "count": known.len(),
    }))
}

/// `load_last_selected_model_index` (`agent.py:8735-8742`): the file's
/// stripped text as an int, or `None` — with the same warning the source
/// logs for both a missing number and an unreadable file.
fn load_index(path: &Path) -> Option<i64> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return None; // no file: the source's `path.exists()` guard, no log
    };
    match text.trim().parse::<i64>() {
        Ok(index) => Some(index),
        Err(error) => {
            tracing::warn!("Failed to load last model index: {error}");
            None
        }
    }
}

/// `save_last_selected_model_index` (`agent.py:8728-8732`): the digits and
/// nothing else — `write_text(str(index))`, no newline. A failure is a
/// logged warning and an `ok` response, exactly as the source's bare
/// `except` answers: the picker keeps the choice in memory either way, and
/// only the restart loses it.
fn save_index(path: &Path, index: i64) {
    if let Err(error) = std::fs::write(path, index.to_string()) {
        tracing::warn!("Failed to save last model index: {error}");
    }
}
