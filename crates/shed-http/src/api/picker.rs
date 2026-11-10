//! The picker's three reads: the countries it offers, the voices behind
//! them, and one country's characters (`agent.py:8482-8542`).
//!
//! The frozen client calls these in sequence — countries on first paint,
//! voices next, characters when a country is chosen — so the payload keys
//! are the contract the browser is built against: `countries[]`, then
//! `voices[]` with `default_voice`/`default_provider`/`provider`/
//! `language`, then `characters[]` with `country`/`language`. Each also
//! answers `ok: true`, which is how the client tells a payload from the
//! error envelope it never sees on a healthy path.

use axum::body::Body;
use axum::extract::{Query, State};
use axum::response::Response;
use serde_json::{json, Value};

use shed_tts::{choose_tts_voice, list_available_voices};

use crate::api::Args;
use crate::json::jsonify;
use crate::state::AppState;

/// `GET /api/countries` — `api_countries` (`agent.py:8482-8491`).
///
/// The names come from the filenames, so a country exists the moment its
/// `*_characters.json` file does; nothing here is configured.
pub async fn countries(State(state): State<AppState>) -> Response<Body> {
    jsonify(&json!({
        "ok": true,
        "countries": state.characters.country_list(),
    }))
}

/// `GET /api/voices` — `api_voices` (`agent.py:8494-8513`).
///
/// `force_refresh=bool(request.args.get("refresh"))` (`agent.py:8499`) has
/// no effect here: the catalog is the frozen `voices.json` snapshot, with
/// nothing to re-fetch until Stage 6 replaces it with the live Edge and
/// Windows lookups. The parameter is still accepted, because the client
/// sends it and rejecting a query the server ignores would be a new failure
/// mode.
///
/// The `default_voice`/`default_provider` pair is what the client
/// preselects, and both are `""` when the catalog offers nothing — which is
/// what a machine with no voices sees in the source too, because its
/// `if voices else None` guard is what the `Err`-on-empty inside
/// `choose_tts_voice` stands in for.
pub async fn voices(State(state): State<AppState>, Query(args): Query<Args>) -> Response<Body> {
    let language = args.language();
    let provider = state.config.tts.provider.as_str();
    let voices = list_available_voices(provider, &language);
    let default = voices.first().and_then(|_| {
        choose_tts_voice(
            provider,
            &state.config.tts.default_voice,
            None,
            None,
            &language,
        )
        .ok()
    });
    jsonify(&json!({
        "ok": true,
        "voices": voices,
        "default_voice": field_text(default.as_ref(), "name"),
        "default_provider": field_text(default.as_ref(), "provider"),
        "provider": provider,
        "language": language,
    }))
}

/// `GET /api/characters` — `api_characters` (`agent.py:8516-8542`).
///
/// No country is not an error but the empty roster: the client clears the
/// list rather than reporting a failure, and the source answers before
/// touching the disk (`agent.py:8526-8527`), which this does too.
pub async fn characters(State(state): State<AppState>, Query(args): Query<Args>) -> Response<Body> {
    let language = args.language();
    let country = args.trimmed(&args.country);
    if country.is_empty() {
        return jsonify(&json!({
            "ok": true,
            "characters": [],
            "country": "",
            "language": language,
        }));
    }

    let voices = list_available_voices(&state.config.tts.provider, &language);
    let characters = state.characters.country_roster(&country, &voices);
    jsonify(&json!({
        "ok": true,
        "characters": characters,
        "country": country,
        "language": language,
    }))
}

/// `default.get(key, "")` for the chosen voice: the catalog's entries carry
/// strings under exactly these keys, and anything else — no chosen voice,
/// or a key a hand-edited snapshot left out — is the empty string the
/// source's `if default_voice else ""` yields.
fn field_text(voice: Option<&Value>, key: &str) -> String {
    voice
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
