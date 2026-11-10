//! The voices the app speaks with, ranked.
//!
//! Ported from `agent.py:3082` (`voice_matches_language`), `agent.py:3097`
//! (`VOICE_TIER_WEIGHT`), `agent.py:3115` (`accent_preference`), `agent.py:3141`
//! (`voice_preference_score`), `agent.py:3222` (`list_available_voices`) and
//! `agent.py:3248` (`choose_tts_voice`). Two of the plan's `shed-tts` lines are
//! shared helpers instead: the voice-depth coercion (`agent.py:2405`, `3267`,
//! `3275`) lives in `shed_text::voice_depth` and the tier classification
//! (`agent.py:3092`, `3100`) lives in `shed_text::voice_tier`, because
//! `shed-characters` validates pitches and mints roster tiers through the same
//! rules and the layer map keeps the two crates apart. Both are re-exported
//! below.
//!
//! The catalog itself is the frozen `voices.json` snapshot at the repository
//! root — 323 voices in exactly the shape `list_edge_voices` and
//! `list_windows_voices` mint at runtime — rather than a live Edge fetch, per
//! the Stage 3 divergence row in `docs/PARITY.md`. `list_available_voices`
//! therefore filters and sorts the snapshot instead of querying providers;
//! the provider filter keeps the exact `agent.py:3226`/`3232` conditions so
//! the answers a live catalog would give are the answers this one gives.

use std::sync::OnceLock;

use serde_json::Value;
use shed_text::{normalize_browser_language, normalize_locale, py_str, text_or_empty};
use tracing::warn;

/// The snapshot the catalog is built from. Frozen by `tests/assets.manifest.json`,
/// so the tests below that pin its size and shape are checking this exact
/// file, and the asset gate fails the build if it changes.
const SNAPSHOT_JSON: &str = include_str!("../../../voices.json");

/// `VOICE_TIER_WEIGHT` (`agent.py:3097`): wide enough that the engine
/// outranks any accent preference below it.
pub const VOICE_TIER_WEIGHT: i64 = 500;

/// The snapshot's voices, parsed once per process.
///
/// The stored `score` fields are **not** trusted: they were stamped by the
/// `score_voice` formula of commit 7864f77 (2026-05-28), which
/// `voice_preference_score` replaced in c5fa1bf (2026-09-21) — the snapshot
/// predates the formula it would have to answer for. Python stamps scores at
/// fetch time (`agent.py:3213`, `agent.py:3180`) and no shipped data path
/// reads a stored score (`/api/voices` only echoes what the catalog holds),
/// so the clone stamps them here with the ported current formula, and the
/// stored numbers are overwritten exactly as a live fetch would overwrite
/// them. The `docs/PARITY.md` Stage-3 divergence row records this.
fn snapshot() -> &'static [Value] {
    static VOICES: OnceLock<Vec<Value>> = OnceLock::new();
    VOICES.get_or_init(|| match serde_json::from_str::<Value>(SNAPSHOT_JSON) {
        Ok(document) => {
            let mut voices = document
                .get("voices")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for voice in &mut voices {
                let score = voice_preference_score(voice);
                if let Some(entry) = voice.as_object_mut() {
                    entry.insert("score".to_owned(), Value::from(score));
                }
            }
            voices
        }
        Err(exc) => {
            warn!("voice catalog: voices.json could not be read: {exc}");
            Vec::new()
        }
    })
}

/// `str(voice.get(key) or "")` (`agent.py:3102`): the catalog's falsy values
/// collapse to the empty string exactly as Python's `or` does, so the
/// falsy rules live once, in `shed_text::text_or_empty`.
fn voice_field(voice: &Value, key: &str) -> String {
    text_or_empty(voice.get(key))
}

/// `str(voice.get(key, ""))` (`agent.py:3118`): only a *missing* key defaults —
/// a key present with `null` stringifies to `"None"`, exactly as Python does.
fn voice_field_or_missing(voice: &Value, key: &str) -> String {
    match voice.get(key) {
        None => String::new(),
        Some(value) => py_str(value),
    }
}

/// `voice_matches_language` (`agent.py:3082`): does this voice speak the
/// language a browser preference collapses to?
#[must_use]
pub fn voice_matches_language(voice: &Value, preferred_language: &str) -> bool {
    let culture = voice_field(voice, "culture");
    let locale_source = if culture.is_empty() {
        voice_field(voice, "locale")
    } else {
        culture
    };
    let locale = normalize_locale(&locale_source);
    let preferred = normalize_browser_language(preferred_language);
    let preferred_prefix = preferred.split('-').next().unwrap_or("");
    locale == preferred_prefix || locale.starts_with(&format!("{preferred_prefix}-"))
}

/// `voice_tier` (`agent.py:3100`), whose home is `shed_text::voice_tier` for
/// the reason in the crate comment; re-exported so the crate the plan names
/// for these lines is the crate that answers for them.
pub use shed_text::{voice_tier, VOICE_TIER_LEGACY, VOICE_TIER_MULTILINGUAL, VOICE_TIER_NEURAL};

/// `accent_preference` (`agent.py:3115`): a taste for some English accents,
/// used only to separate voices of one tier.
#[must_use]
pub fn accent_preference(voice: &Value) -> i64 {
    let text = [
        "name",
        "culture",
        "gender",
        "description",
        "locale",
        "friendly_name",
    ]
    .iter()
    .map(|key| voice_field_or_missing(voice, key))
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    let mut score = 0;
    if text.contains("en-gb")
        || text.contains("united kingdom")
        || text.contains("great britain")
        || text.contains("british")
    {
        score += 160;
    }
    if text.contains("en-au") || text.contains("australia") || text.contains("australian") {
        score += 130;
    }
    if text.contains("en-nz") || text.contains("new zealand") {
        score += 120;
    }
    if text.contains("en-ie") || text.contains("ireland") || text.contains("irish") {
        score += 110;
    }
    if text.contains("en-ca") || text.contains("canada") || text.contains("canadian") {
        score += 70;
    }
    if text.contains("english") {
        score += 20;
    }
    if text.contains("en-us") || text.contains("united states") || text.contains("american") {
        score -= 20;
    }
    score
}

/// `voice_preference_score` (`agent.py:3141`): within a language, the better
/// engine first, then the accent.
#[must_use]
pub fn voice_preference_score(voice: &Value) -> i64 {
    voice_tier(voice) * VOICE_TIER_WEIGHT + accent_preference(voice)
}

/// The `item.get("score", 0)` of a catalog entry; a score that is not a
/// number reads as 0, the way `.get("score", 0)` would hand a non-number
/// straight to the comparison it loses in.
fn score_of(voice: &Value) -> f64 {
    voice.get("score").and_then(Value::as_f64).unwrap_or(0.0)
}

/// `list_available_voices` (`agent.py:3222`), reading the snapshot instead of
/// the live providers.
///
/// The two provider conditions are `agent.py:3226` and `agent.py:3232`
/// verbatim: `edge` pulls in the edge voices *and* the windows ones, and an
/// unrecognised provider name ships nothing, exactly as a machine whose every
/// provider raised would. Because the snapshot cannot raise, the
/// `RuntimeError` at `agent.py:3238` has no trigger here and is not ported.
#[must_use]
pub fn list_available_voices(provider: &str, preferred_language: &str) -> Vec<Value> {
    let include_edge = provider == "edge" || provider == "auto";
    let include_windows = matches!(provider, "windows" | "auto" | "edge");
    let mut voices: Vec<Value> = snapshot()
        .iter()
        .filter(|voice| {
            let voice_provider = voice_field(voice, "provider");
            (include_edge && voice_provider == "edge")
                || (include_windows && voice_provider == "windows")
        })
        .cloned()
        .collect();
    // Python's `sorted(..., key=(matches, score), reverse=True)` is stable:
    // ties keep catalog order. `sort_by` is stable too, so reversing only the
    // comparison reproduces it.
    voices.sort_by(|a, b| {
        let key_a = (voice_matches_language(a, preferred_language), score_of(a));
        let key_b = (voice_matches_language(b, preferred_language), score_of(b));
        key_b
            .partial_cmp(&key_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    voices
}

/// `choose_tts_voice` (`agent.py:3248`) with the two settings the source reads
/// as module globals — `TTS_PROVIDER` and `TTS_DEFAULT_VOICE` (`agent.py:557`,
/// `agent.py:558`) — passed in, as every other setting is in this clone.
///
/// The `Err` strings are the source's `RuntimeError` messages verbatim; the
/// stage that serves them turns them into response bodies.
pub fn choose_tts_voice(
    provider: &str,
    default_voice: &str,
    requested_voice: Option<&str>,
    requested_provider: Option<&str>,
    preferred_language: &str,
) -> Result<Value, String> {
    let voices = list_available_voices(provider, preferred_language);
    let Some(first) = voices.first() else {
        return Err("No TTS voices are available".to_owned());
    };

    if let Some(requested) = requested_voice.filter(|name| !name.is_empty()) {
        for voice in &voices {
            let name_matches = voice.get("name").is_some_and(|name| name == requested);
            let provider_matches = requested_provider.unwrap_or("").is_empty()
                || voice
                    .get("provider")
                    .is_some_and(|p| p == requested_provider.unwrap_or(""));
            if name_matches && provider_matches {
                return Ok(voice.clone());
            }
        }
        let provider_label = requested_provider
            .filter(|p| !p.is_empty())
            .unwrap_or("any");
        return Err(format!("Voice not found: {provider_label}:{requested}"));
    }

    if !default_voice.is_empty() {
        for voice in &voices {
            let name_matches = voice.get("name").is_some_and(|name| name == default_voice);
            let provider_matches =
                provider == "auto" || voice.get("provider").is_some_and(|p| p == provider);
            if name_matches && provider_matches {
                return Ok(voice.clone());
            }
        }
    }

    Ok(first.clone())
}

/// Voice-depth coercion lives in [`shed_text::voice_depth`] because
/// `shed-characters` validates authored pitches through the same rule; the
/// re-export keeps the synthesis code's import in one place.
pub use shed_text::{coerce_f64, coerce_voice_depth, normalize_voice_depth};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn voice(name: &str, fields: &[(&str, Value)]) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("name".to_owned(), json!(name));
        for (key, value) in fields {
            map.insert((*key).to_owned(), value.clone());
        }
        Value::Object(map)
    }

    fn named<'a>(voices: impl IntoIterator<Item = &'a Value>, key: &str) -> Vec<String> {
        voices
            .into_iter()
            .map(|v| v.get(key).and_then(Value::as_str).unwrap_or("").to_owned())
            .collect()
    }

    #[test]
    fn accents_separate_voices_within_one_tier() {
        // `accent_preference` (`agent.py:3115`): British first, American last.
        assert_eq!(accent_preference(&voice("en-GB-Foo", &[])), 160);
        assert_eq!(
            accent_preference(&voice("Foo", &[("description", json!("United Kingdom"))])),
            160
        );
        assert_eq!(accent_preference(&voice("en-AU-Foo", &[])), 130);
        assert_eq!(accent_preference(&voice("en-NZ-Foo", &[])), 120);
        assert_eq!(accent_preference(&voice("en-IE-Foo", &[])), 110);
        assert_eq!(accent_preference(&voice("en-CA-Foo", &[])), 70);
        assert_eq!(accent_preference(&voice("en-US-Foo", &[])), -20);
        // "English (American)" earns the language bonus and loses the region one.
        assert_eq!(
            accent_preference(&voice("x", &[("description", json!("English (American)"))])),
            0
        );
        assert_eq!(accent_preference(&voice("Plain", &[])), 0);
    }

    #[test]
    fn preference_score_is_tier_times_weight_plus_accent() {
        let ryan = voice(
            "en-GB-RyanNeural",
            &[
                ("culture", json!("en-GB")),
                ("description", json!("English (British)")),
            ],
        );
        assert_eq!(
            voice_preference_score(&ryan),
            VOICE_TIER_NEURAL * VOICE_TIER_WEIGHT + 160 + 20
        );
    }

    #[test]
    fn missing_keys_default_but_null_stringifies_like_python() {
        // `voice.get(key, "")` on a present null yields "None" (`agent.py:3118`).
        let with_null = voice("x", &[("culture", json!(null))]);
        assert!(accent_preference(&with_null) >= 0); // "none" carries no accent words
                                                     // `voice.get(key) or ""` collapses falsy values (`agent.py:3103`).
        let falsy = voice(
            "x",
            &[("display_name", json!(false)), ("description", json!(0))],
        );
        assert_eq!(voice_tier(&falsy), 0);
    }

    #[test]
    fn language_match_compares_the_voice_language_not_the_locale() {
        let british = voice("x", &[("culture", json!("en-GB"))]);
        assert!(voice_matches_language(&british, "en-US"));
        assert!(voice_matches_language(&british, "en"));
        assert!(!voice_matches_language(&british, "fr-FR"));
        // culture falls back to locale, both fall back to "" (`agent.py:3084`).
        let locale_only = voice("x", &[("locale", json!("fr_FR"))]);
        assert!(voice_matches_language(&locale_only, "fr-CA"));
        let blank = voice("x", &[]);
        // An empty voice language never matches, not even the en-GB fallback.
        assert!(!voice_matches_language(&blank, ""));
    }

    #[test]
    fn snapshot_holds_the_frozen_file_and_rewrites_every_score() {
        let voices = list_available_voices("auto", "");
        assert_eq!(voices.len(), 323);
        let edge = voices
            .iter()
            .filter(|v| v.get("provider") == Some(&json!("edge")))
            .count();
        assert_eq!(edge, 321);
        assert_eq!(voices.len() - edge, 2);

        // The stored scores came from the formula of commit 7864f77; the
        // catalog restates every one of them with the current formula, so a
        // voice whose two formulas disagree proves the rewrite happened.
        let raw: Value = serde_json::from_str(SNAPSHOT_JSON).unwrap_or_default();
        let stored = raw
            .get("voices")
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|v| v.get("name") == Some(&json!("cy-GB-AledNeural")))
            })
            .and_then(|v| v.get("score"));
        assert_eq!(stored, Some(&json!(258)));

        let aled = voices
            .iter()
            .find(|v| v.get("name") == Some(&json!("cy-GB-AledNeural")));
        assert_eq!(aled.and_then(|v| v.get("score")), Some(&json!(660))); // tier 1 + "United Kingdom" 160
    }

    #[test]
    fn provider_filter_keeps_the_sources_provider_sets() {
        // `agent.py:3226`/`agent.py:3232`: edge means edge+windows, windows
        // means windows alone, anything else ships nothing.
        assert_eq!(list_available_voices("auto", "").len(), 323);
        assert_eq!(list_available_voices("edge", "").len(), 323);
        let windows = list_available_voices("windows", "");
        assert_eq!(
            named(&windows, "name"),
            ["Microsoft David Desktop", "Microsoft Zira Desktop"]
        );
        assert!(list_available_voices("bogus", "").is_empty());
    }

    #[test]
    fn preferred_language_rises_first_and_ties_keep_snapshot_order() {
        // Python's stable `sorted(key=(matches, score), reverse=True)` over
        // the snapshot: French voices first for a French listener, then the
        // best-scored English ones, ties in snapshot order.
        let french = list_available_voices("auto", "fr-FR");
        assert_eq!(
            named(&french[..3], "name"),
            [
                "fr-FR-RemyMultilingualNeural",
                "fr-FR-VivienneMultilingualNeural",
                "fr-CA-ThierryNeural"
            ]
        );

        let british = list_available_voices("auto", "en-GB");
        assert_eq!(
            named(&british[..5], "name"),
            [
                "en-AU-WilliamMultilingualNeural",
                "en-US-AndrewMultilingualNeural",
                "en-US-BrianMultilingualNeural",
                "en-US-AvaMultilingualNeural",
                "en-US-EmmaMultilingualNeural",
            ]
        );
        // The four 1000-point en-US multilingual voices tie, so their relative
        // order is the snapshot's own — a stable sort, not a re-shuffle.
        assert_eq!(british[1].get("score"), Some(&json!(1000)));
        assert_eq!(british[4].get("score"), Some(&json!(1000)));
    }

    /// The name of the voice a `choose_tts_voice` answer carries.
    fn chosen_name(result: Result<Value, String>) -> Option<String> {
        result
            .ok()
            .and_then(|voice| voice.get("name").and_then(Value::as_str).map(str::to_owned))
    }

    #[test]
    fn choose_prefers_the_requested_voice_then_the_default_then_the_best() {
        let denise = choose_tts_voice(
            "auto",
            "en-GB-RyanNeural",
            Some("fr-FR-DeniseNeural"),
            None,
            "fr-FR",
        );
        assert_eq!(chosen_name(denise).as_deref(), Some("fr-FR-DeniseNeural"));

        // The default voice wins over the best-scoring voice.
        let ryan = choose_tts_voice("auto", "en-GB-RyanNeural", None, None, "en-GB");
        assert_eq!(chosen_name(ryan).as_deref(), Some("en-GB-RyanNeural"));

        // No default: the best voice for the language answers.
        let best = choose_tts_voice("auto", "", None, None, "en-GB");
        assert_eq!(
            chosen_name(best).as_deref(),
            Some("en-AU-WilliamMultilingualNeural")
        );

        // A default whose provider does not match the catalog's provider is
        // skipped (`agent.py:3261`), falling through to voices[0].
        let david = choose_tts_voice("windows", "en-GB-RyanNeural", None, None, "en-GB");
        assert_eq!(
            chosen_name(david).as_deref(),
            Some("Microsoft David Desktop")
        );
    }

    #[test]
    fn choose_reports_the_sources_runtime_errors_verbatim() {
        let empty = choose_tts_voice("bogus", "en-GB-RyanNeural", None, None, "");
        assert_eq!(empty.err().as_deref(), Some("No TTS voices are available"));

        let missing = choose_tts_voice("auto", "", Some("no-such-voice"), None, "");
        assert_eq!(
            missing.err().as_deref(),
            Some("Voice not found: any:no-such-voice")
        );

        // A provider that does not own the voice reads as not found, naming
        // the provider that was asked for.
        let wrong = choose_tts_voice("auto", "", Some("Microsoft Zira Desktop"), Some("edge"), "");
        assert_eq!(
            wrong.err().as_deref(),
            Some("Voice not found: edge:Microsoft Zira Desktop")
        );
    }
}
