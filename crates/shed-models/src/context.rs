//! The context window behind a model, and the label the model picker shows for it.
//!
//! Ported from `agent.py:6408` (`_extract_model_context_length`), `6439`
//! (`resolve_model_context_window`), `6467` (`format_model_context_length`) and
//! `6482` (`format_model_display_name`) — the part of the `agent.py:6207-6497`
//! region (§3.2) that `GET /api/models` (`agent.py:8545`) reads, and the part
//! the chat stage reads again when it sizes a request.
//!
//! ## The record is typed, so the key scan is not
//!
//! `_extract_model_context_length` walks eight key names across a record and
//! across two sub-objects of it (`config`, `model`). Every writer of
//! `MODEL_METADATA` builds the record through `ingest_discovery_item`
//! (`agent.py:1476-1485`) or through `_normalize_model_metadata_map` reading
//! that same shape back from the cache file, and both carry exactly one context
//! key: `context_length`. The other seven names and the two sub-objects are
//! therefore unreachable for a record this app owns, which is what lets Stage 2
//! keep the record a struct ([`crate::ModelMetadata`]) and this module read one
//! field (`docs/PARITY.md`, "Structural changes").
//!
//! ## The cap is a literal
//!
//! `agent.py:6450` caps every window at a hardcoded `32768`, not at
//! `LM_ENGINE_NUM_CTX`: the configured window is what gets capped, never the
//! cap. A setting that says 131072 still resolves to `min(131072, 32768)`. The
//! clone copies that rather than repairing it, because the label the picker
//! shows is derived from the number it produces.
//!
//! ## The `or` chain loses one rung
//!
//! The source's `MODEL_METADATA.get(a) or MODEL_METADATA.get(b) or {}` also
//! falls through a record that exists but is an empty dict, which is the only
//! shape Python can hold and the typed record cannot. The clone falls through a
//! *missing* key and keeps a record that exists (`docs/PARITY.md`,
//! "Structural changes").

use shed_config::parse::normalize_model_name;
use shed_config::AppConfig;

use crate::catalog::{ModelCatalog, ModelMetadata};

/// `agent.py:6450`'s cap: the window every model is held to unless its file is
/// large enough to need the smaller one. A literal in the source, so a literal
/// here — see the module header.
const FALLBACK_CONTEXT_CAP: i64 = 32_768;

/// The settings the resolver reads, borrowed from the loaded configuration:
/// `AI_MODEL` (`agent.py:398-402` region's default model) and the three context
/// settings.
#[derive(Debug, Clone)]
pub struct ContextSettings {
    /// `AI_MODEL`: what an empty or blank model name resolves to before the
    /// record is looked up (`normalize_AI_MODEL_name`'s default).
    pub ai_model: String,
    /// `LM_ENGINE_NUM_CTX`: the window assumed for a record that carries none.
    pub num_ctx: i64,
    /// `LM_ENGINE_LARGE_MODEL_CTX`: the cap for a model over the size threshold.
    pub large_model_ctx: i64,
    /// `LM_ENGINE_LARGE_MODEL_SIZE_GB`: the threshold itself, in gigabytes.
    pub large_model_size_gb: f64,
}

impl ContextSettings {
    /// The four reads the resolver needs, from the one place they are configured.
    #[must_use]
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            ai_model: config.ai_model.clone(),
            num_ctx: config.lm.num_ctx,
            large_model_ctx: config.lm.large_model_ctx,
            large_model_size_gb: config.lm.large_model_size_gb,
        }
    }

    /// `int(LM_ENGINE_LARGE_MODEL_SIZE_GB * (1024 ** 3))` (`agent.py:6451`):
    /// gigabytes to bytes, truncated the way `int()` truncates a float.
    fn large_model_size_bytes(&self) -> i64 {
        (self.large_model_size_gb * 1_073_741_824.0).trunc() as i64
    }
}

/// `_extract_model_context_length` (`agent.py:6408`) on a typed record: the one
/// context key it can carry, and only when it is positive — the source's
/// `if parsed > 0` is what makes a record that says `0` (or a negative) read as
/// "the payload said nothing" rather than as a window of that size.
fn discovered_context(metadata: Option<&ModelMetadata>) -> i64 {
    metadata
        .and_then(|record| record.context_length)
        .filter(|parsed| *parsed > 0)
        .unwrap_or(0)
}

/// `MODEL_METADATA.get(name) or MODEL_METADATA.get(name.split("@", 1)[0]) or {}`
/// (`agent.py:6442`, `6489`): a quantization label is not a model of its own, so
/// the window comes from the base record.
///
/// The `or` runs on *existence* here rather than on emptiness: the typed record
/// has no empty shape, so a record that exists is kept where Python would have
/// fallen through an empty dict (`docs/PARITY.md`, "Structural changes").
fn record_for<'a>(catalog: &'a ModelCatalog, name: &str) -> Option<&'a ModelMetadata> {
    catalog
        .metadata_for(name)
        .or_else(|| catalog.metadata_for(name.split('@').next().unwrap_or(name)))
}

/// The tail of `resolve_model_context_window` once the record is in hand
/// (`agent.py:6443-6464`): the cap first, then the discovered window, then the
/// configured one.
fn window_from_metadata(metadata: Option<&ModelMetadata>, settings: &ContextSettings) -> i64 {
    let size_bytes = metadata.map_or(0, |record| record.size);
    let mut context_cap = FALLBACK_CONTEXT_CAP;
    if size_bytes > settings.large_model_size_bytes() {
        context_cap = settings.large_model_ctx;
    }

    let discovered = discovered_context(metadata);
    if discovered > 0 {
        return discovered.min(context_cap);
    }

    // The source returns `configured_ctx` on both paths below, so a configured
    // window of 0 is 0 rather than "no answer" — a caller that wants to tell
    // them apart (`agent.py:6864`) is the one that clamps.
    let configured = settings.num_ctx;
    if configured > 0 {
        return configured.min(context_cap);
    }
    configured
}

/// `resolve_model_context_window` (`agent.py:6439`): the token window to plan a
/// request around, capped for a model whose file is large enough to need it.
///
/// `None` and a blank name both resolve through `AI_MODEL`, which is
/// `normalize_AI_MODEL_name`'s `str(value or "").strip() or default`.
///
/// Crate-private until its own caller arrives: `agent.py`'s two external readers
/// are `format_model_display_name` right here and `call_lm_engine` (6864), which
/// is Stage 4 (`How_To_Code` principle 16).
#[must_use]
pub(crate) fn resolve_model_context_window(
    catalog: &ModelCatalog,
    model_name: Option<&str>,
    settings: &ContextSettings,
) -> i64 {
    let name = normalize_model_name(model_name.unwrap_or_default(), &settings.ai_model);
    window_from_metadata(record_for(catalog, &name), settings)
}

/// `format_model_context_length` (`agent.py:6467`): the window as the picker
/// writes it — `4k`, `8k`, `20k`.
///
/// Never public, in the source or here: its only reader is `format_model_display_name`
/// directly below, which is the only thing in the app that shows a window.
///
/// The last branch is where Python's `round` shows: it rounds halves to even,
/// so `2560` is `2k` and `4608` is `4k` rather than `3k` and `5k`. Rust's
/// `round` would round half away from zero and say otherwise, so the clone uses
/// `round_ties_even`, which is the same rule on the same binary value.
#[must_use]
pub(crate) fn format_model_context_length(context_length: i64) -> String {
    if context_length <= 0 {
        return String::new();
    }
    if context_length % 1024 == 0 {
        return format!("{}k", context_length / 1024);
    }
    if context_length % 1000 == 0 {
        return format!("{}k", context_length / 1000);
    }
    let thousands = (context_length as f64 / 1024.0).round_ties_even();
    format!("{}k", (thousands as i64).max(1))
}

/// `format_model_display_name` (`agent.py:6482`): a model's name with its
/// window appended, unless it has no window, no label, or already says it.
///
/// `display_name` is the record's own display; an empty one falls back to
/// `model_name`, which is the `display_name or model_name or ""` the source
/// writes at the call site (`agent.py:8552`) and inside this function.
#[must_use]
pub fn format_model_display_name(
    catalog: &ModelCatalog,
    model_name: Option<&str>,
    display_name: &str,
    settings: &ContextSettings,
) -> String {
    let chosen = if display_name.is_empty() {
        model_name.unwrap_or_default()
    } else {
        display_name
    };
    let base_display = chosen.trim();
    if base_display.is_empty() {
        return String::new();
    }

    let name = normalize_model_name(model_name.unwrap_or_default(), &settings.ai_model);
    let metadata = record_for(catalog, &name);
    if discovered_context(metadata) <= 0 {
        return base_display.to_string();
    }

    let context_label =
        format_model_context_length(resolve_model_context_window(catalog, model_name, settings));
    if context_label.is_empty() || base_display.ends_with(&format!(" ({context_label})")) {
        return base_display.to_string();
    }
    format!("{base_display} ({context_label})")
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn settings(ai_model: &str) -> ContextSettings {
        ContextSettings {
            ai_model: ai_model.to_owned(),
            num_ctx: 32_768,
            large_model_ctx: 16_384,
            large_model_size_gb: 12.0,
        }
    }

    /// A catalog with exactly these models, in this order, each record built by
    /// the same `normalize_metadata_map` the discovery pass uses, from the JSON
    /// that pass would have been handed.
    fn catalog(entries: &[(&str, Value)]) -> ModelCatalog {
        let ordered: Vec<String> = entries
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect();
        let mut raw = serde_json::Map::new();
        for (name, value) in entries {
            raw.insert((*name).to_string(), (*value).clone());
        }
        let metadata = crate::catalog::normalize_metadata_map(&Value::Object(raw), &ordered);
        let mut catalog = ModelCatalog::default();
        catalog.apply_known_models(&ordered, metadata);
        catalog
    }

    #[test]
    fn a_record_with_a_window_resolves_it_unchanged_when_small() {
        let catalog = catalog(&[("gemma", serde_json::json!({"context_length": 8192}))]);
        assert_eq!(
            resolve_model_context_window(&catalog, Some("gemma"), &settings("gemma")),
            8_192
        );
    }

    #[test]
    fn a_window_over_the_cap_is_capped_not_rejected() {
        let catalog = catalog(&[("gemma", serde_json::json!({"context_length": 131_072}))]);
        assert_eq!(
            resolve_model_context_window(&catalog, Some("gemma"), &settings("gemma")),
            FALLBACK_CONTEXT_CAP,
            "the cap is the source's literal 32768, whatever LM_ENGINE_NUM_CTX says"
        );
    }

    #[test]
    fn a_large_model_gets_the_reduced_cap() {
        let large_bytes = (12.0f64 * 1_073_741_824.0).trunc() as i64;
        let just_over = catalog(&[(
            "big",
            serde_json::json!({"context_length": 131_072, "size": large_bytes + 1}),
        )]);
        assert_eq!(
            resolve_model_context_window(&just_over, Some("big"), &settings("big")),
            16_384,
            "one byte over the threshold is a large model"
        );

        let exactly = catalog(&[(
            "big",
            serde_json::json!({"context_length": 131_072, "size": large_bytes}),
        )]);
        assert_eq!(
            resolve_model_context_window(&exactly, Some("big"), &settings("big")),
            FALLBACK_CONTEXT_CAP,
            "the source compares with >, so a model of exactly the threshold size is not large"
        );
    }

    #[test]
    fn no_record_and_no_window_fall_back_to_the_configured_context() {
        let catalog = catalog(&[("gemma", serde_json::json!({"display": "Gemma"}))]);
        let mut config = settings("gemma");
        config.num_ctx = 131_072;
        assert_eq!(
            resolve_model_context_window(&catalog, Some("gemma"), &config),
            FALLBACK_CONTEXT_CAP,
            "the configured window is capped too"
        );

        config.num_ctx = 4_096;
        assert_eq!(
            resolve_model_context_window(&catalog, Some("gemma"), &config),
            4_096
        );

        config.num_ctx = 0;
        assert_eq!(
            resolve_model_context_window(&catalog, Some("gemma"), &config),
            0,
            "LM_ENGINE_NUM_CTX=0 is the source's own no-window answer"
        );
    }

    #[test]
    fn a_blank_name_resolves_through_ai_model() {
        let catalog = catalog(&[("gemma", serde_json::json!({"context_length": 8_192}))]);
        assert_eq!(
            resolve_model_context_window(&catalog, None, &settings("gemma")),
            8_192,
            "None and blank both mean AI_MODEL"
        );
        assert_eq!(
            resolve_model_context_window(&catalog, Some("   "), &settings("gemma")),
            8_192
        );
    }

    #[test]
    fn a_variant_name_finds_its_base_record() {
        // The `@` half of the name is not a model of its own: the base record is
        // what carries the window, so the lookup falls through to it.
        let base_only = catalog(&[("gemma", serde_json::json!({"context_length": 8_192}))]);
        assert_eq!(
            resolve_model_context_window(&base_only, Some("gemma@4bit"), &settings("gemma")),
            8_192
        );

        // A variant that has a record of its own keeps it, even though that record
        // says nothing: the source's `or` falls through a *missing* key, and
        // `{display: "gemma@4bit"}` is a truthy dict it stops at.
        let variant_recorded = catalog(&[
            ("gemma@4bit", serde_json::json!({})),
            ("gemma", serde_json::json!({"context_length": 8_192})),
        ]);
        assert_eq!(
            resolve_model_context_window(&variant_recorded, Some("gemma@4bit"), &settings("gemma")),
            32_768,
            "so the window comes from LM_ENGINE_NUM_CTX, not from the base record"
        );
    }

    #[test]
    fn a_registry_latest_name_loses_its_tag_before_the_lookup() {
        let catalog = catalog(&[(
            "registry:5000/gemma",
            serde_json::json!({"context_length": 8_192}),
        )]);
        assert_eq!(
            resolve_model_context_window(
                &catalog,
                Some("registry:5000/gemma:latest"),
                &settings("unused")
            ),
            8_192,
            "normalize_AI_MODEL_name drops :latest from a registry name first"
        );
    }

    #[test]
    fn a_zero_or_negative_record_window_reads_as_no_window() {
        let zero = catalog(&[("gemma", serde_json::json!({"context_length": 0}))]);
        let negative = catalog(&[("gemma", serde_json::json!({"context_length": -5}))]);
        for record in [&zero, &negative] {
            assert_eq!(
                resolve_model_context_window(record, Some("gemma"), &settings("gemma")),
                32_768,
                "the source's `if parsed > 0` makes both fall through to LM_ENGINE_NUM_CTX"
            );
        }
    }

    #[test]
    fn context_lengths_are_labelled_in_thousands() {
        for (length, label) in [
            (4_096, "4k"),
            (8_192, "8k"),
            (32_768, "32k"),
            (2_048, "2k"),
            (20_000, "20k"),
            (5_000, "5k"),
            (1_000_000, "1000k"),
        ] {
            assert_eq!(format_model_context_length(length), label, "{length}");
        }
    }

    #[test]
    fn a_length_that_is_not_a_round_thousand_is_rounded() {
        for (length, label) in [
            (4_000, "4k"),
            (4_300, "4k"),
            (100, "1k"),
            (512, "1k"),
            (1_536, "2k"),
            (3_584, "4k"),
        ] {
            assert_eq!(format_model_context_length(length), label, "{length}");
        }
    }

    /// Python rounds a half to the nearest *even* integer, so 2560 tokens is
    /// `2k` and 4608 is `4k`. Both are exactly `.5` of a kilobyte-window and
    /// both are the values a real model reports (`2.5k` is nothing, but the
    /// rounding rule is what a test can pin).
    #[test]
    fn halves_round_to_even_the_way_python_round_does() {
        assert_eq!(format_model_context_length(2_560), "2k");
        assert_eq!(format_model_context_length(4_608), "4k");
        assert_eq!(format_model_context_length(6_656), "6k");
        assert_eq!(
            format_model_context_length(512),
            "1k",
            "max(1, 0) is still 1k"
        );
    }

    #[test]
    fn no_window_is_no_label() {
        assert_eq!(format_model_context_length(0), "");
        assert_eq!(format_model_context_length(-4_096), "");
    }

    #[test]
    fn a_display_name_carries_its_window_once() {
        let catalog = catalog(&[("gemma", serde_json::json!({"context_length": 4_096}))]);
        let config = settings("gemma");
        assert_eq!(
            format_model_display_name(&catalog, Some("gemma"), "Gemma 3", &config),
            "Gemma 3 (4k)"
        );
        assert_eq!(
            format_model_display_name(&catalog, Some("gemma"), "Gemma 3 (4k)", &config),
            "Gemma 3 (4k)",
            "a display that already says it is not labelled twice"
        );
        assert_eq!(
            format_model_display_name(&catalog, Some("gemma"), "Gemma 3 (8k)", &config),
            "Gemma 3 (8k) (4k)",
            "a stale label is not corrected: the source only tests for the exact suffix it would add"
        );
    }

    #[test]
    fn a_model_without_a_window_is_named_as_it_arrives() {
        let catalog = catalog(&[("gemma", serde_json::json!({"display": "Gemma 3"}))]);
        assert_eq!(
            format_model_display_name(&catalog, Some("gemma"), "Gemma 3", &settings("gemma")),
            "Gemma 3"
        );
    }

    #[test]
    fn an_empty_display_falls_back_to_the_model_name() {
        let catalog = catalog(&[("gemma", serde_json::json!({}))]);
        assert_eq!(
            format_model_display_name(&catalog, Some("gemma"), "", &settings("gemma")),
            "gemma"
        );
        assert_eq!(
            format_model_display_name(&catalog, Some("  "), "", &settings("gemma")),
            "",
            "display_name or model_name or empty, then stripped"
        );
    }

    /// The picker's own loop (`agent.py:8550-8553`): every known model, in the
    /// catalog's order, each labelled from its own record.
    #[test]
    fn the_picker_labels_every_known_model_in_order() {
        let catalog = catalog(&[
            ("small", serde_json::json!({"context_length": 4_096})),
            ("plain", serde_json::json!({"display": "Plain"})),
        ]);
        let config = settings("small");
        let labels: Vec<String> = catalog
            .known_models()
            .iter()
            .map(|name| {
                let display = catalog
                    .metadata_for(name)
                    .map_or(name.as_str(), |record| record.display.as_str());
                format_model_display_name(&catalog, Some(name), display, &config)
            })
            .collect();
        assert_eq!(labels, vec!["small (4k)".to_owned(), "Plain".to_owned()]);
    }
}
