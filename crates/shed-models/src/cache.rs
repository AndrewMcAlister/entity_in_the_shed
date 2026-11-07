//! `STORAGE_ROOT/models_cache.json`: the last discovery the app managed to make, kept so the next
//! boot can say something before the engine answers.
//!
//! Ported from `agent.py:1267` (`_load_cached_model_discovery`) and `1292`
//! (`_persist_model_discovery_cache`).
//!
//! The path is not a setting: the source hard-codes `STORAGE_ROOT / "models_cache.json"` at 1268
//! and 1297, so the storage root is handed in and the file name is fixed here.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use tracing::debug;

use crate::catalog::{normalize_metadata_map, ModelMetadata};

/// The file's name inside the storage root.
pub const CACHE_FILE_NAME: &str = "models_cache.json";

/// `_load_cached_model_discovery`: the catalog from the last successful discovery, or nothing when
/// there is no cache to read.
///
/// A cache that cannot be read is not an error the app acts on — it is a debug line and an empty
/// result, after which discovery falls through to the disk scan.
pub(crate) fn load_cached_model_discovery(
    storage_root: &Path,
) -> (Vec<String>, BTreeMap<String, ModelMetadata>) {
    let cache_path = storage_root.join(CACHE_FILE_NAME);
    if !cache_path.is_file() {
        return (Vec::new(), BTreeMap::new());
    }

    match read_cache(&cache_path) {
        Ok((ordered, metadata)) => (ordered, metadata),
        Err(reason) => {
            debug!(
                "Failed to read models cache from {}: {reason}",
                cache_path.display()
            );
            (Vec::new(), BTreeMap::new())
        }
    }
}

fn read_cache(cache_path: &Path) -> Result<(Vec<String>, BTreeMap<String, ModelMetadata>), String> {
    let text = std::fs::read_to_string(cache_path).map_err(|error| error.to_string())?;
    let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;

    // `payload.get(...)` on anything that is not a dict raises in Python, and the `except` turns it
    // into the same empty result a broken file gets.
    let payload = payload
        .as_object()
        .ok_or("the cache is not a JSON object")?;
    let candidates = payload.get("known_models").and_then(Value::as_array);

    let mut ordered = Vec::new();
    let mut seen = Vec::new();
    for item in candidates.unwrap_or(&Vec::new()) {
        let model_key = match item {
            Value::String(text) => text.trim().to_string(),
            Value::Null => String::new(),
            other => other.to_string().trim().to_string(),
        };
        if model_key.is_empty() || seen.iter().any(|seen| seen == &model_key) {
            continue;
        }
        seen.push(model_key.clone());
        ordered.push(model_key);
    }

    let metadata = payload
        .get("model_metadata")
        .cloned()
        .unwrap_or(Value::Null);
    let metadata = normalize_metadata_map(&metadata, &ordered);
    Ok((ordered, metadata))
}

/// `_persist_model_discovery_cache`: write the catalog back, as the two things a fallback actually
/// needs.
///
/// The cache is deliberately minimal (`agent.py:1298-1304`): a model key list and one display name
/// per key. A record's other fields are not written, and `_load_cached_model_discovery` does not
/// miss them — it rebuilds every record from what is there. An empty catalog is not written at all,
/// so a failed discovery cannot erase a good cache.
pub(crate) fn persist_model_discovery_cache(
    storage_root: &Path,
    ordered: &[String],
    metadata: &BTreeMap<String, ModelMetadata>,
) {
    if ordered.is_empty() {
        return;
    }

    let cache_path = storage_root.join(CACHE_FILE_NAME);
    match write_cache(&cache_path, ordered, metadata) {
        Ok(()) => debug!("Wrote models cache to {}", cache_path.display()),
        Err(reason) => debug!("Failed to write models cache: {reason}"),
    }
}

fn write_cache(
    cache_path: &Path,
    ordered: &[String],
    metadata: &BTreeMap<String, ModelMetadata>,
) -> Result<(), String> {
    let mut minimal = Map::new();
    minimal.insert(
        "known_models".to_string(),
        Value::Array(
            ordered
                .iter()
                .map(|key| Value::String(key.clone()))
                .collect(),
        ),
    );

    let mut records = Map::new();
    for key in ordered {
        let display = metadata
            .get(key)
            .map(|record| Value::String(record.display.clone()))
            .unwrap_or(Value::Null);
        let mut record = Map::new();
        record.insert("display".to_string(), display);
        records.insert(key.clone(), Value::Object(record));
    }
    minimal.insert("model_metadata".to_string(), Value::Object(records));

    // `json.dump(..., indent=2, ensure_ascii=False)` (`agent.py:1306`): two-space indent, non-ASCII
    // written as itself, and no trailing newline. The source opens the file in text mode, so its
    // line endings are CRLF on Windows; the clone writes LF (`docs/PARITY.md`, "Structural
    // changes").
    let text =
        serde_json::to_string_pretty(&Value::Object(minimal)).map_err(|error| error.to_string())?;
    std::fs::write(cache_path, text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn ordered(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// A directory of its own under the approved temporary tree, so the cache has somewhere to be
    /// written and nothing the app owns is created inside the repo.
    fn scratch_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("shed-models-cache-tests")
            .join(label);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("creating {} must work: {error}", root.display()));
        root
    }

    #[test]
    fn a_missing_cache_is_no_models_and_no_warning() {
        let root = scratch_root("missing");
        let (models, metadata) = load_cached_model_discovery(&root);
        assert!(models.is_empty());
        assert!(metadata.is_empty());
    }

    #[test]
    fn a_written_cache_is_read_back_as_the_same_catalog() {
        let root = scratch_root("round-trip");
        let models = ordered(&["qwen3.8-flash-next-iq3_xxs", "llama-3.1-8b"]);
        let mut metadata = BTreeMap::new();
        for key in &models {
            metadata.insert(
                key.clone(),
                ModelMetadata {
                    display: format!("{key} (q4_0)"),
                    downloaded: Some(true),
                    ..ModelMetadata::default()
                },
            );
        }

        persist_model_discovery_cache(&root, &models, &metadata);
        let (restored, restored_metadata) = load_cached_model_discovery(&root);
        assert_eq!(restored, models);
        // Only the display name survives a round trip, which is all a fallback needs.
        assert_eq!(
            restored_metadata["qwen3.8-flash-next-iq3_xxs"].display,
            "qwen3.8-flash-next-iq3_xxs (q4_0)"
        );
        assert_eq!(
            restored_metadata["qwen3.8-flash-next-iq3_xxs"].downloaded,
            None
        );
    }

    #[test]
    fn the_cache_holds_only_the_keys_the_source_writes() {
        let root = scratch_root("shape");
        persist_model_discovery_cache(&root, &ordered(&["a"]), &BTreeMap::new());
        let text = std::fs::read_to_string(root.join(CACHE_FILE_NAME))
            .unwrap_or_else(|error| panic!("reading the cache: {error}"));
        assert_eq!(
            text,
            "{\n  \"known_models\": [\n    \"a\"\n  ],\n  \"model_metadata\": {\n    \"a\": {\n      \"display\": null\n    }\n  }\n}"
        );
    }

    #[test]
    fn an_empty_catalog_does_not_erase_the_cache() {
        let root = scratch_root("empty-catalog");
        persist_model_discovery_cache(&root, &ordered(&["a"]), &BTreeMap::new());
        persist_model_discovery_cache(&root, &[], &BTreeMap::new());
        let (models, _) = load_cached_model_discovery(&root);
        assert_eq!(models, ordered(&["a"]));
    }

    #[test]
    fn a_cache_that_is_not_an_object_is_no_models() {
        let root = scratch_root("not-an-object");
        std::fs::write(root.join(CACHE_FILE_NAME), "[]")
            .unwrap_or_else(|error| panic!("writing the fixture: {error}"));
        let (models, metadata) = load_cached_model_discovery(&root);
        assert!(models.is_empty());
        assert!(metadata.is_empty());
    }

    #[test]
    fn a_cache_that_is_not_json_is_no_models() {
        let root = scratch_root("not-json");
        std::fs::write(root.join(CACHE_FILE_NAME), "{ not json")
            .unwrap_or_else(|error| panic!("writing the fixture: {error}"));
        let (models, _) = load_cached_model_discovery(&root);
        assert!(models.is_empty());
    }

    #[test]
    fn blank_and_repeated_names_in_a_cache_are_dropped() {
        let root = scratch_root("blank-names");
        let fixture = json!({"known_models": [" a ", "a", "", "  ", "b"]});
        std::fs::write(root.join(CACHE_FILE_NAME), fixture.to_string())
            .unwrap_or_else(|error| panic!("writing the fixture: {error}"));
        let (models, _) = load_cached_model_discovery(&root);
        assert_eq!(models, ordered(&["a", "b"]));
    }
}
