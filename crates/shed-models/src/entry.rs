//! One candidate entry the discovery pass collected, and the two rules that decide which of them
//! reach the catalog and in what order.
//!
//! Ported from `utils/models.py:134` (`filter_embedding_model_entries`) and `152`
//! (`model_discovery_size_sort_key`), applied to the lists `agent.py:1544-1545` builds.

use std::collections::BTreeMap;

use crate::catalog::ModelMetadata;
use crate::size::parse_model_size_billions;

/// One entry of `models_info` / `all_models_info` (`agent.py:1490`, `1513`, `1524`, `1527`).
///
/// `model_type` is `None` on the entries a `selected_variant` produced: those are appended without
/// a `type` key (1524-1527), which is what keeps them out of the embedding filter's reach.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEntry {
    pub key: String,
    pub size: i64,
    pub display: String,
    pub params: Option<f64>,
    pub model_type: Option<String>,
}

/// `filter_embedding_model_entries` (`utils/models.py:134`): a model the engine called an embedding
/// model is a catalog entry but not something a conversation can run on.
///
/// When filtering would empty the list, the source hands back the unfiltered list (147) — an engine
/// that labels every model `embedding` still gets a catalog rather than an empty one. That is
/// reproduced here, because the caller treats an empty catalog as "discovery found nothing" and
/// falls back to the cache.
pub(crate) fn filter_embedding_model_entries(
    source_models: &[ModelEntry],
    discovered_metadata: &BTreeMap<String, ModelMetadata>,
) -> Vec<ModelEntry> {
    let filtered: Vec<ModelEntry> = source_models
        .iter()
        .filter(|entry| {
            if is_embedding(entry.model_type.as_deref()) {
                return false;
            }
            // 142-145: an entry that does not carry a type is asked of the metadata instead.
            let metadata_type = discovered_metadata
                .get(&entry.key)
                .and_then(|record| record.model_type.as_deref());
            !is_embedding(metadata_type)
        })
        .cloned()
        .collect();

    if filtered.is_empty() {
        return source_models.to_vec();
    }
    filtered
}

/// The comparison the source makes twice, at `utils/models.py:141` and `145`: `str(x).lower() ==
/// "embedding"`. Ingest already lowercases a type it read from a payload; a type that arrived from
/// the cache has not been through ingest, so the comparison is case-insensitive here.
fn is_embedding(model_type: Option<&str>) -> bool {
    model_type.is_some_and(|model_type| model_type.eq_ignore_ascii_case("embedding"))
}

/// `model_discovery_size_sort_key` (`utils/models.py:152`): the parameter count the engine stated,
/// or the billions written in the name, or the byte count, and infinity when none of the three is
/// known.
///
/// Infinity is not a size, it is the statement that this model goes last.
pub(crate) fn size_sort_key(entry: &ModelEntry) -> f64 {
    if let Some(params) = entry.params {
        return params;
    }

    let name = if entry.key.is_empty() {
        &entry.display
    } else {
        &entry.key
    };
    let parsed = parse_model_size_billions(name);
    if parsed != f64::INFINITY {
        return parsed;
    }

    if entry.size > 0 {
        return entry.size as f64;
    }
    f64::INFINITY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, params: Option<f64>, size: i64, model_type: Option<&str>) -> ModelEntry {
        ModelEntry {
            key: key.to_string(),
            size,
            display: key.to_string(),
            params,
            model_type: model_type.map(|model_type| model_type.to_string()),
        }
    }

    fn metadata_with_type(key: &str, model_type: &str) -> BTreeMap<String, ModelMetadata> {
        let mut records = BTreeMap::new();
        records.insert(
            key.to_string(),
            ModelMetadata {
                display: key.to_string(),
                model_type: Some(model_type.to_string()),
                ..ModelMetadata::default()
            },
        );
        records
    }

    #[test]
    fn an_explicit_embedding_entry_is_left_out_of_the_chat_list() {
        let source = vec![
            entry("chat", None, 0, None),
            entry("embed", None, 0, Some("embedding")),
        ];
        let filtered = filter_embedding_model_entries(&source, &BTreeMap::new());
        let keys: Vec<&str> = filtered.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys, ["chat"]);
    }

    #[test]
    fn an_entry_without_a_type_is_asked_of_the_metadata() {
        let source = vec![entry("embed", None, 0, None)];
        let records = metadata_with_type("embed", "embedding");
        // Filtering empties the list, so the source — unfiltered — comes back.
        assert_eq!(filter_embedding_model_entries(&source, &records), source);
    }

    #[test]
    fn a_chat_model_the_metadata_calls_an_embedding_is_dropped_when_another_model_survives() {
        let source = vec![entry("chat", None, 0, None), entry("embed", None, 0, None)];
        let records = metadata_with_type("embed", "Embedding");
        let filtered = filter_embedding_model_entries(&source, &records);
        let keys: Vec<&str> = filtered.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys, ["chat"]);
    }

    #[test]
    fn a_type_spelled_in_any_case_is_an_embedding() {
        let source = vec![
            entry("a", None, 0, Some("EMBEDDING")),
            entry("b", None, 0, Some("llm")),
        ];
        let filtered = filter_embedding_model_entries(&source, &BTreeMap::new());
        assert_eq!(filtered, vec![entry("b", None, 0, Some("llm"))]);
    }

    #[test]
    fn params_come_first() {
        assert_eq!(
            size_sort_key(&entry("unknown", Some(7.0), 999_999, None)),
            7.0
        );
    }

    #[test]
    fn then_the_size_written_in_the_name() {
        assert_eq!(size_sort_key(&entry("llama-3.1-8b", None, 0, None)), 8.0);
    }

    #[test]
    fn then_the_byte_count() {
        assert_eq!(
            size_sort_key(&entry("unknown", None, 12_345, None)),
            12_345.0
        );
    }

    #[test]
    fn and_last_a_model_nothing_measured() {
        assert_eq!(
            size_sort_key(&entry("unknown", None, 0, None)),
            f64::INFINITY
        );
        assert_eq!(
            size_sort_key(&entry("unknown", None, -5, None)),
            f64::INFINITY
        );
    }

    #[test]
    fn a_blank_key_asks_the_display_name_instead() {
        let mut unnamed = entry("", None, 0, None);
        unnamed.display = "mistral-7b".to_string();
        assert_eq!(size_sort_key(&unnamed), 7.0);
    }
}
