//! The catalog: what the app believes is on the machine, and the record it keeps for each model.
//!
//! Ported from `agent.py:1239-1259` (`KNOWN_MODELS`, `MODEL_METADATA`, `_apply_known_models`) and
//! `utils/models.py:21` (`_normalize_model_metadata_map`).
//!
//! The source keeps those three as module globals that any function in `agent.py` can reach. Here
//! they are one struct that a stage mutates and passes to whoever asks, which is also why
//! `MODEL_PATHS` is absent: its value is always `{name: name}` and its only readers (6835-6858)
//! are inside `call_lm_engine`, so it arrives at Stage 4 as a method rather than as a third global.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

/// One model's record: `MODEL_METADATA[key]` (`agent.py:1241`).
///
/// Python builds this dict with a different set of keys on each path that writes it, and this
/// struct carries every key it ever uses so the paths can be written as assignments rather than as
/// merge rules. A key the source does not set on a given path is `None` here, which is what the
/// source's missing key means to every reader: `metadata.get("type")` is `None` there, and
/// `metadata.model_type` is `None` here.
///
/// The branch that drops keys is reproduced rather than repaired — `agent.py:1514-1519` replaces a
/// downloaded chat model's entry with only `size`, `display`, `variants` and `selected_variant`, so
/// `context_length`, `params`, `type` and `downloaded` are lost for exactly the models Stage 3's
/// `resolve_model_context_window` will read. That is the state the Python app runs with, and the
/// clone keeps it: repairing it would change which context length the app resolves.
///
/// `model_type` rather than `type`, because `type` is a Rust keyword.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelMetadata {
    /// `size` — the byte count the discovery payload reported, 0 when it reported none.
    pub size: i64,
    /// `display` — the name shown to a person, quantization suffix included when known.
    pub display: String,
    /// `variants` — the quantizations the engine listed for this model.
    pub variants: Vec<String>,
    /// `selected_variant` — the one the engine has picked, which is also a load target.
    pub selected_variant: Option<String>,
    /// `downloaded` — whether the model is on disk. `None` where the source omits the key.
    pub downloaded: Option<bool>,
    /// `context_length` — the token window, where a payload said it.
    pub context_length: Option<i64>,
    /// `params` — the parameter count in billions, parsed out of `params_string`.
    pub params: Option<f64>,
    /// `type` — `embedding`, `llm`, or whatever else the engine called it.
    pub model_type: Option<String>,
    /// `source` — `disk` for an entry the directory scan produced.
    pub source: Option<String>,
}

impl ModelMetadata {
    /// `{display: model_key}` — the entry the source writes when it knows a model exists but knows
    /// nothing else about it (`utils/models.py:31`, `34`).
    fn display_only(model_key: &str) -> Self {
        Self {
            display: model_key.to_string(),
            ..Self::default()
        }
    }
}

/// `MODEL_METADATA` and `KNOWN_MODELS` together: the order is load-bearing, because
/// `resolve_AI_MODEL` (`agent.py:1737`) takes the first entry and the discovery sort put the
/// smallest model there.
#[derive(Clone, Debug, Default)]
pub struct ModelCatalog {
    pub(crate) known_models: Vec<String>,
    pub(crate) metadata: BTreeMap<String, ModelMetadata>,
}

impl ModelCatalog {
    /// `_apply_known_models` (`agent.py:1244`): replace what the app believes, keeping the order it
    /// was handed and dropping blank and repeated names.
    ///
    /// Two parts of the source function are not here. `MODEL_PATHS` (1258) waits for its first
    /// reader at Stage 4. The browser tail (1260-1264) is not ported at all: `BROWSER_MODEL_OPTIONS`
    /// and `BROWSER_DEFAULT_MODEL` are written here and again at 2192-2193 and read by nothing but
    /// tests, so the clone keeps one catalog rather than two copies of it
    /// (`docs/PARITY.md`, "Surfaces the clone does not carry").
    pub(crate) fn apply_known_models(
        &mut self,
        ordered: &[String],
        mut metadata: BTreeMap<String, ModelMetadata>,
    ) {
        let mut normalized_order = Vec::new();
        let mut seen = BTreeSet::new();
        for name in ordered {
            let model_key = name.trim();
            if model_key.is_empty() || !seen.insert(model_key.to_string()) {
                continue;
            }
            normalized_order.push(model_key.to_string());
        }

        // 1254, and the `setdefault` pass inside it (utils/models.py:33-34): every catalog entry
        // has a record, even one no discovery path ever described.
        for model_key in &normalized_order {
            metadata
                .entry(model_key.clone())
                .or_insert_with(|| ModelMetadata::display_only(model_key));
        }

        self.known_models = normalized_order;
        self.metadata = metadata;
    }

    /// The catalog as the app currently holds it, for the fallback the source snapshots before it
    /// tries to learn anything new (`agent.py:1539-1540`).
    pub(crate) fn snapshot(&self) -> (Vec<String>, BTreeMap<String, ModelMetadata>) {
        (self.known_models.clone(), self.metadata.clone())
    }

    /// `KNOWN_MODELS` in the order the discovery pass sorted it — smallest model first, which is
    /// the order the model picker shows them in (`agent.py:8550`).
    #[must_use]
    pub fn known_models(&self) -> &[String] {
        &self.known_models
    }

    /// `MODEL_METADATA.get(name)`: `None` is the state the source's `or` chains fall through, so
    /// callers that reproduce one of those chains (`crate::context::record_for`, the picker's
    /// `meta.get("display") or name`) do it on this `Option` rather than on an empty record.
    #[must_use]
    pub fn metadata_for(&self, name: &str) -> Option<&ModelMetadata> {
        self.metadata.get(name)
    }
}

/// `_normalize_model_metadata_map` (`utils/models.py:21`), reading a JSON object rather than a
/// Python dict: blank keys are dropped, a value that is not an object becomes `{display: key}`, and
/// a model that is in the catalog but has no record gets one.
pub(crate) fn normalize_metadata_map(
    metadata: &Value,
    ordered: &[String],
) -> BTreeMap<String, ModelMetadata> {
    let mut normalized = BTreeMap::new();
    if let Some(entries) = metadata.as_object() {
        for (key, value) in entries {
            let model_key = key.trim();
            if model_key.is_empty() {
                continue;
            }
            normalized.insert(model_key.to_string(), metadata_from_value(model_key, value));
        }
    }

    for model_key in ordered {
        normalized
            .entry(model_key.clone())
            .or_insert_with(|| ModelMetadata::display_only(model_key));
    }
    normalized
}

/// One record out of a JSON object: the keys this struct knows, and nothing else.
///
/// The source copies the whole dict (`utils/models.py:29`), so a payload that invented a tenth key
/// would keep it there and lose it here. Only the keys the app ever reads are carried, which is
/// what makes the record a struct at all (`docs/PARITY.md`, "Structural changes").
fn metadata_from_value(model_key: &str, value: &Value) -> ModelMetadata {
    let Some(entry) = value.as_object() else {
        return ModelMetadata::display_only(model_key);
    };

    ModelMetadata {
        size: entry
            .get("size")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        display: entry
            .get("display")
            .and_then(Value::as_str)
            .unwrap_or(model_key)
            .to_string(),
        variants: string_list(entry.get("variants")),
        selected_variant: text(entry.get("selected_variant")),
        downloaded: entry.get("downloaded").and_then(Value::as_bool),
        context_length: entry.get("context_length").and_then(Value::as_i64),
        params: entry.get("params").and_then(Value::as_f64),
        model_type: text(entry.get("type")),
        source: text(entry.get("source")),
    }
}

/// The value of a key, as the text the source would have written with `str(...)`, when it is text
/// at all. A boolean is `True`/`False` rather than `true`/`false`, because that is what
/// `str(True)` says in Python.
pub(crate) fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Bool(true)) => Some("True".to_string()),
        Some(Value::Bool(false)) => Some("False".to_string()),
        _ => None,
    }
}

/// `variants` as the source keeps them: a list whose entries are copied verbatim, anything that is
/// not a list as an empty one (`agent.py:1375-1377`).
pub(crate) fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(entries)) => entries
            .iter()
            .map(|entry| match entry {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Python's truthiness for a JSON value: `null`, `false`, a blank string, a zero, an empty list and
/// an empty object are false, which is what every `x or y` in this module tests.
pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::String(text) => !text.is_empty(),
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::Array(entries) => !entries.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// `item.get(a) or item.get(b) or fallback`, where a JSON `null`, `false`, a blank string, a zero,
/// an empty list or an empty object is Python's false.
pub(crate) fn first_present<'a>(item: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| item.get(*key))
        .find(|value| is_truthy(value))
}

/// `int(value or 0)` (`agent.py:1373`): a number is truncated toward zero the way `int()` truncates
/// one, and anything that is not a number is 0.
///
/// The source does not guard this conversion at all, so a payload that spelled a size as `"many"`
/// raised `ValueError` all the way out of `discover_lm_studio_models` and took the request with it.
/// The clone reads it as no size instead (`docs/PARITY.md`, "Structural changes").
pub(crate) fn as_int(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_f64().unwrap_or_default().trunc() as i64,
        Some(Value::Bool(true)) => 1,
        Some(Value::String(text)) => text.trim().parse::<i64>().unwrap_or_default(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ordered(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn object_values_are_kept() {
        let result = normalize_metadata_map(&json!({"a": {"display": "A"}}), &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result["a"].display, "A");
    }

    #[test]
    fn a_value_that_is_not_an_object_becomes_the_key_as_display() {
        // `{"b": "B"}` becomes `{"b": {"display": "b"}}` there: the display is the key, not the
        // value the source threw away.
        let result = normalize_metadata_map(&json!({"b": "B"}), &[]);
        assert_eq!(result["b"], ModelMetadata::display_only("b"));
    }

    #[test]
    fn blank_keys_are_skipped() {
        let result = normalize_metadata_map(&json!({"": "x", "   ": "y"}), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn catalog_models_without_a_record_get_one() {
        let result = normalize_metadata_map(&json!({}), &ordered(&["z", "y"]));
        assert_eq!(result.len(), 2);
        assert_eq!(result["z"].display, "z");
        assert_eq!(result["y"].display, "y");
    }

    #[test]
    fn an_existing_record_is_not_replaced() {
        let result = normalize_metadata_map(&json!({"c": {"display": "custom"}}), &ordered(&["c"]));
        assert_eq!(result["c"].display, "custom");
    }

    #[test]
    fn a_payload_that_is_not_an_object_has_no_records() {
        assert!(normalize_metadata_map(&json!("weird"), &[]).is_empty());
        assert!(normalize_metadata_map(&Value::Null, &[]).is_empty());
    }

    #[test]
    fn applying_a_catalog_strips_and_dedupes_names_but_keeps_the_order() {
        let mut catalog = ModelCatalog::default();
        catalog.apply_known_models(&ordered(&["  a  ", "a", "", "  b"]), BTreeMap::new());
        assert_eq!(catalog.known_models, ordered(&["a", "b"]));
        assert_eq!(catalog.metadata.len(), 2);
    }

    #[test]
    fn a_record_for_a_model_that_is_not_in_the_catalog_is_kept() {
        // `_normalize_model_metadata_map` copies every entry it is given (utils/models.py:24-31) and
        // `_apply_known_models` assigns the result straight to `MODEL_METADATA` (1259), so a record
        // whose model is not in the order survives. That is how `embed` stays described in
        // `MODEL_METADATA` after a discovery that left it out of `KNOWN_MODELS`.
        let mut metadata = BTreeMap::new();
        metadata.insert("ghost".to_string(), ModelMetadata::display_only("ghost"));
        let mut catalog = ModelCatalog::default();
        catalog.apply_known_models(&ordered(&["a"]), metadata);
        assert_eq!(catalog.known_models, ordered(&["a"]));
        assert!(catalog.metadata.contains_key("ghost"));
    }

    #[test]
    fn a_record_keeps_the_fields_the_source_kept_and_no_others() {
        let record = metadata_from_value(
            "qwen",
            &json!({
                "size": 4096,
                "display": "Qwen (q4_0)",
                "variants": ["q4_0", {"file": "x"}],
                "selected_variant": "q4_0",
                "downloaded": true,
                "context_length": 8192,
                "params": 7.0,
                "type": "llm",
                "source": "disk",
            }),
        );
        assert_eq!(record.size, 4096);
        assert_eq!(record.display, "Qwen (q4_0)");
        // A variant that is not a string keeps its JSON text rather than being dropped.
        assert_eq!(record.variants, ["q4_0", "{\"file\":\"x\"}"]);
        assert_eq!(record.selected_variant.as_deref(), Some("q4_0"));
        assert_eq!(record.downloaded, Some(true));
        assert_eq!(record.context_length, Some(8192));
        assert_eq!(record.params, Some(7.0));
        assert_eq!(record.model_type.as_deref(), Some("llm"));
        assert_eq!(record.source.as_deref(), Some("disk"));
    }

    #[test]
    fn python_false_is_not_a_present_value() {
        let payload = json!({"key": "", "id": "found", "size_bytes": 0, "state": null});
        let item = payload
            .as_object()
            .unwrap_or_else(|| panic!("the fixture must be an object"));
        assert_eq!(
            first_present(item, &["key", "id"]).and_then(Value::as_str),
            Some("found")
        );
        assert_eq!(first_present(item, &["size_bytes", "size"]), None);
        assert_eq!(first_present(item, &["state", "missing"]), None);
    }

    #[test]
    fn a_size_is_read_the_way_int_reads_it() {
        assert_eq!(as_int(Some(&json!(4096))), 4096);
        assert_eq!(as_int(Some(&json!(1.9))), 1, "int() truncates toward zero");
        assert_eq!(as_int(Some(&json!(" 12 "))), 12);
        assert_eq!(as_int(Some(&json!(true))), 1);
        assert_eq!(as_int(Some(&json!("many"))), 0);
        assert_eq!(as_int(None), 0);
    }
}
