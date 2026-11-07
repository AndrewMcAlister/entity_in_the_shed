//! Turning one engine's answer into the catalog: the ingest rules, the candidate shapes, and the
//! fallback chain that decides what the app believes when the answer is empty.
//!
//! Ported from `agent.py:1354` (`ingest_discovery_item`), `utils/models.py:117`
//! (`parse_discovery_candidates`) and the catalog half of `agent.py:1529`
//! (`discover_lm_studio_models`). The HTTP half of that function is `shed-llm`'s, because
//! plan §3.3 lets `shed-llm` reach `shed-models` and never the reverse.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use tracing::{debug, info, warn};

use crate::cache::{load_cached_model_discovery, persist_model_discovery_cache};
use crate::catalog::{
    as_int, first_present, is_truthy, string_list, text, ModelCatalog, ModelMetadata,
};
use crate::disk::discover_models_from_disk;
use crate::entry::{filter_embedding_model_entries, size_sort_key, ModelEntry};

/// The keys `ingest_discovery_item` accepts as a model's name (`agent.py:1363-1371`), in the order
/// it tries them. LM Studio has never agreed with itself about which one it uses.
const KEY_NAMES: [&str; 6] = ["key", "id", "identifier", "model", "name", "llm"];

/// The keys a context length hides under (`agent.py:1455`, `1466`).
const CONTEXT_KEYS: [&str; 6] = [
    "context_length",
    "n_ctx",
    "context_size",
    "max_context",
    "max_context_length",
    "context_window",
];

/// `parse_discovery_candidates` (`utils/models.py:117`): the shapes an engine's model list has been
/// seen in.
pub(crate) fn parse_discovery_candidates(data: &Value) -> Vec<Value> {
    if let Some(object) = data.as_object() {
        if let Some(entries) = object.get("models").and_then(Value::as_array) {
            return entries.clone();
        }
    }

    if let Some(entries) = data.as_array() {
        return entries.clone();
    }

    if let Some(object) = data.as_object() {
        for key in ["data", "available", "items", "list", "models"] {
            if let Some(entries) = object.get(key).and_then(Value::as_array) {
                return entries.clone();
            }
        }
    }

    Vec::new()
}

/// `ingest_discovery_item` (`agent.py:1354`): one candidate entry, into the three lists the
/// discovery pass builds.
///
/// Every quirk of the source is kept, including the two that look accidental: a downloaded
/// chat-capable model's metadata is *replaced* with four keys (1514-1519), losing the context length
/// and the type that were just computed for it; and a model with a `selected_variant` is appended to
/// the full catalog list a second time (1490 and 1527), which the later dedupe quietly removes.
pub(crate) fn ingest_discovery_item(
    item: &Value,
    discovered_metadata: &mut BTreeMap<String, ModelMetadata>,
    models_info: &mut Vec<ModelEntry>,
    all_models_info: &mut Vec<ModelEntry>,
) {
    let Some(candidate) = candidate_identity(item) else {
        return;
    };
    let facts = read_candidate_facts(item);
    let key = candidate.key.clone();
    let display = candidate.display_with_quantization.as_str();
    let size = candidate.size;

    // 1476-1485: `setdefault`, so the first entry for a key is the one whose record survives.
    discovered_metadata
        .entry(key.clone())
        .or_insert_with(|| full_record(&candidate, &facts));

    // 1490: the full list holds catalog-only models too, which is what makes it the fallback when
    // nothing is marked downloaded.
    all_models_info.push(entry_of(&candidate, &facts));

    if !facts.is_downloaded {
        debug!("Skipping catalog-only model {key} (not downloaded)");
        return;
    }

    if facts
        .model_type
        .as_deref()
        .is_some_and(|model_type| model_type == "embedding")
    {
        debug!("Skipping embedding-only model for chat list: {key}");
        // 1503-1511: an update of the record the `setdefault` wrote, so the type is recorded on a
        // model that stays out of the chat list.
        let record = discovered_metadata.entry(key.clone()).or_default();
        record.size = size;
        record.display = display.to_string();
        record.variants = candidate.variants.clone();
        record.selected_variant = candidate.selected_variant.clone();
        record.model_type = facts.model_type.clone();
    } else {
        models_info.push(entry_of(&candidate, &facts));
        // 1514-1519: the four-key replacement, which drops `downloaded`, `context_length`, `params`
        // and `type` in the source and here.
        discovered_metadata.insert(key.clone(), replacement_record(&candidate));
    }

    append_selected_variant(
        &key,
        size,
        display,
        facts.params,
        discovered_metadata,
        models_info,
        all_models_info,
    );
}

/// The list entry one candidate produces: 1490 and 1502 build it from the same candidate and the
/// same `params`/`type`, so the two pushes are the same entry.
fn entry_of(candidate: &Candidate, facts: &Facts) -> ModelEntry {
    entry_for(
        &candidate.key,
        candidate.size,
        &candidate.display_with_quantization,
        facts.params,
        facts.model_type.clone(),
    )
}

/// The record `setdefault` writes: every key the source lists at 1476-1485.
fn full_record(candidate: &Candidate, facts: &Facts) -> ModelMetadata {
    ModelMetadata {
        size: candidate.size,
        display: candidate.display_with_quantization.clone(),
        variants: candidate.variants.clone(),
        selected_variant: candidate.selected_variant.clone(),
        downloaded: Some(facts.is_downloaded),
        context_length: facts.context_length,
        params: facts.params,
        model_type: facts.model_type.clone(),
        source: None,
    }
}

/// The record the else branch assigns at 1514-1519: four keys, and nothing else.
fn replacement_record(candidate: &Candidate) -> ModelMetadata {
    ModelMetadata {
        size: candidate.size,
        display: candidate.display_with_quantization.clone(),
        variants: candidate.variants.clone(),
        selected_variant: candidate.selected_variant.clone(),
        ..ModelMetadata::default()
    }
}

/// 1522-1527: the variant the engine selected is itself a load target, so it goes on the lists as
/// its own model, with no `type` key on either entry.
///
/// The variant is read back out of the metadata rather than out of the payload, because the two
/// branches above have just written it there — and for a downloaded chat model, that write is the
/// four-key replacement, which is the only reason the value is still present at all.
fn append_selected_variant(
    key: &str,
    size: i64,
    display: &str,
    params: Option<f64>,
    discovered_metadata: &BTreeMap<String, ModelMetadata>,
    models_info: &mut Vec<ModelEntry>,
    all_models_info: &mut Vec<ModelEntry>,
) {
    let Some(selected) = discovered_metadata
        .get(key)
        .and_then(|record| record.selected_variant.clone())
        .filter(|selected| !selected.is_empty())
    else {
        return;
    };

    models_info.push(entry_for(&selected, size, &selected, params, None));
    all_models_info.push(entry_for(&selected, size, &selected, params, None));
    // 1527 appends the model itself a second time, which the dedupe at 1591 quietly removes.
    all_models_info.push(entry_for(key, size, display, params, None));
}

/// What the payload says about a candidate, read once.
struct Facts {
    is_downloaded: bool,
    params: Option<f64>,
    model_type: Option<String>,
    context_length: Option<i64>,
}

/// The four readings that depend on the payload's shape rather than on the candidate's identity:
/// 1385-1399, 1419-1434, 1436-1448 and 1451-1474.
fn read_candidate_facts(item: &Value) -> Facts {
    Facts {
        is_downloaded: candidate_downloaded(item),
        params: candidate_params(item),
        model_type: candidate_type(item),
        // 1482: `int(ctx) if ctx else None` — a context length of 0 is recorded as no context length.
        context_length: candidate_context_length(item).filter(|context| *context != 0),
    }
}

/// The parts of an ingest that come straight out of the payload: name, size, display, variants.
struct Candidate {
    key: String,
    size: i64,
    variants: Vec<String>,
    display_with_quantization: String,
    selected_variant: Option<String>,
}

fn candidate_identity(item: &Value) -> Option<Candidate> {
    match item {
        // 1357-1361: a bare string is a model name with nothing else known about it, and it counts
        // as downloaded because there is no payload left to say otherwise.
        Value::String(text) => {
            let key = text.trim().to_string();
            if key.is_empty() {
                return None;
            }
            Some(Candidate {
                key: key.clone(),
                size: 0,
                variants: Vec::new(),
                display_with_quantization: key,
                selected_variant: None,
            })
        }
        Value::Object(object) => {
            let key = text(first_present(object, &KEY_NAMES))
                .unwrap_or_default()
                .trim()
                .to_string();
            if key.is_empty() {
                return None;
            }
            let display =
                text(first_present(object, &["display_name"])).unwrap_or_else(|| key.clone());
            let variants = string_list(first_present(object, &["variants", "available_variants"]));
            let selected_variant = text(first_present(object, &["selected_variant"]));
            Some(Candidate {
                key,
                size: as_int(first_present(object, &["size_bytes", "size"])),
                variants,
                display_with_quantization: with_quantization(&display, object),
                selected_variant,
            })
        }
        _ => None,
    }
}

/// 1385-1399: a dict says it is downloaded when it says it is downloaded, or when it has instances,
/// or when a state or a downloaded-variant list says so. None of those can turn a `true` into a
/// `false` once one of them has said `true`, and a bare string starts `true`.
fn candidate_downloaded(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return true;
    };

    let mut downloaded = ["downloaded", "is_downloaded", "isDownloaded"]
        .iter()
        .any(|name| object.get(*name).is_some_and(is_truthy));
    downloaded |= match object.get("instances") {
        Some(Value::Array(instances)) => !instances.is_empty(),
        Some(value) => is_truthy(value),
        None => false,
    };

    let state = text(object.get("state")).unwrap_or_default().to_lowercase();
    if matches!(state.as_str(), "downloaded" | "ready" | "loaded") {
        downloaded = true;
    }

    if let Some(Value::Array(variants)) =
        first_present(object, &["downloaded_variants", "downloadedVariants"])
    {
        downloaded |= !variants.is_empty();
    }

    downloaded
}

/// 1402-1415: `"{display} ({quantization})"`, where the quantization is a nested `name`, one of two
/// camelCase spellings, or a key that contains a literal dot.
fn with_quantization(display: &str, object: &Map<String, Value>) -> String {
    let quantization = match object.get("quantization") {
        Some(Value::Object(nested)) => text(nested.get("name")),
        _ => None,
    };

    let quantization = quantization
        .or_else(|| {
            text(first_present(
                object,
                &["quantization_name", "quantizationName", "quantization.name"],
            ))
        })
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    match quantization {
        Some(quantization) => format!("{display} ({quantization})"),
        None => display.to_string(),
    }
}

/// 1419-1434: the largest `9B`-shaped number in `params_string`, `params` or `params_str`. Only a
/// string is read; a numeric `params` key is ignored here, because `isinstance(pstr, str)` says so.
fn candidate_params(item: &Value) -> Option<f64> {
    let params_text = match item {
        Value::Object(object) => first_present(object, &["params_string", "params", "params_str"]),
        _ => None,
    };

    let Some(Value::String(params_text)) = params_text else {
        return None;
    };

    parse_param_billions(params_text.as_str())
}

/// `re.findall(r"(\d+(?:\.\d+)?)\s*[Bb]", pstr, flags=re.IGNORECASE)` then `max(...)`.
pub(crate) fn parse_param_billions(params_text: &str) -> Option<f64> {
    let mut largest: Option<f64> = None;
    let bytes: Vec<char> = params_text.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        let mut end = index;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == '.' {
            let mut fraction = end + 1;
            while fraction < bytes.len() && bytes[fraction].is_ascii_digit() {
                fraction += 1;
            }
            if fraction > end + 1 {
                end = fraction;
            }
        }

        // `\s*[Bb]`: any run of whitespace, then a B. A digit run that is not followed by one is
        // not a parameter count.
        let mut after = end;
        while after < bytes.len() && bytes[after].is_whitespace() {
            after += 1;
        }
        if after < bytes.len() && (bytes[after] == 'B' || bytes[after] == 'b') {
            if let Ok(value) = bytes[index..end].iter().collect::<String>().parse::<f64>() {
                largest = Some(largest.map_or(value, |largest: f64| largest.max(value)));
            }
        }
        index = end;
    }
    largest
}

/// 1436-1448: the first of four keys that names what kind of model this is, lowercased.
fn candidate_type(item: &Value) -> Option<String> {
    match item {
        Value::Object(object) => first_present(object, &["type", "model_type", "task", "llm_type"])
            .and_then(|value| text(Some(value)))
            .map(|model_type| model_type.trim().to_lowercase()),
        _ => None,
    }
}

/// 1451-1474: a context length at the top level, or under `config`, or under `model`.
///
/// A key that is present but is not a number does not end the search: `int()` raises, the `except`
/// sets `ctx` back to `None`, and the loop moves on to the next key. A key present with a JSON
/// `null` is skipped entirely, which is what `item.get(ctx_key) is not None` means.
fn candidate_context_length(item: &Value) -> Option<i64> {
    let Value::Object(object) = item else {
        return None;
    };

    for key in CONTEXT_KEYS {
        match object.get(key) {
            None | Some(Value::Null) => continue,
            Some(value) => {
                if let Some(context) = context_number(value) {
                    return Some(context);
                }
            }
        }
    }

    // 1463-1474: the nested search only runs when nothing was found above, and it stops at the first
    // parent that produced a usable value.
    for parent_key in ["config", "model"] {
        let Some(parent) = object.get(parent_key).and_then(Value::as_object) else {
            continue;
        };
        for key in CONTEXT_KEYS {
            match parent.get(key) {
                None | Some(Value::Null) => continue,
                Some(value) => {
                    if let Some(context) = context_number(value) {
                        return Some(context);
                    }
                }
            }
        }
    }

    None
}

/// `int(x or 0)` inside the `try` at 1457: a value Python would read as false becomes 0, a number is
/// truncated the way `int()` truncates, and a value `int()` would raise on is no context length.
fn context_number(value: &Value) -> Option<i64> {
    match value {
        Value::Null | Value::Bool(false) => Some(0),
        Value::String(text) if text.is_empty() => Some(0),
        Value::Array(entries) if entries.is_empty() => Some(0),
        Value::Object(entries) if entries.is_empty() => Some(0),
        Value::Bool(true) => Some(1),
        Value::Number(number) => Some(number.as_f64().unwrap_or_default().trunc() as i64),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn entry_for(
    key: &str,
    size: i64,
    display: &str,
    params: Option<f64>,
    model_type: Option<String>,
) -> ModelEntry {
    ModelEntry {
        key: key.to_string(),
        size,
        display: display.to_string(),
        params,
        model_type,
    }
}

/// 1565-1571: every candidate in one engine's answer, ingested into the three lists the pass builds.
///
/// The size the line reports is `len(str(data))` — the parsed document's own text — rather than the
/// bytes the socket carried, which is what the source logs.
fn ingest_candidates(
    payload: &Value,
    url: &str,
    discovered_metadata: &mut BTreeMap<String, ModelMetadata>,
    models_info: &mut Vec<ModelEntry>,
    all_models_info: &mut Vec<ModelEntry>,
) {
    debug!(
        "Model discovery: fetched {} bytes from {url}",
        payload.to_string().len()
    );
    let candidates = parse_discovery_candidates(payload);
    info!(
        "Model discovery: {} candidate entries from {url}",
        candidates.len()
    );
    for item in &candidates {
        ingest_discovery_item(item, discovered_metadata, models_info, all_models_info);
    }
}

/// The catalog half of `discover_lm_studio_models` (`agent.py:1539-1629`): take what an engine
/// answered, and if that is nothing, fall back through what the app already believed, then the
/// cache, then the disk.
///
/// `payload` is `None` when the request failed, when it answered with a status that was not 200, or
/// when the body was not JSON — the three `continue` paths at 1550-1562, which `shed-llm` has
/// already logged. `url` is here only so the two lines that name it say the same thing they said in
/// Python.
///
/// The in-memory fallback is the catalog as it was before this call, which is why the source
/// snapshots it at 1539-1540 before it tries to learn anything. The cache fallback does not write
/// the cache back (1614-1618); the disk fallback does (1624).
pub fn apply_discovered_payload(
    catalog: &mut ModelCatalog,
    payload: Option<&Value>,
    url: &str,
    storage_root: &Path,
    models_root: Option<&Path>,
) -> Vec<String> {
    let (previous_known_models, previous_metadata) = catalog.snapshot();

    let mut discovered_metadata = BTreeMap::new();
    let mut models_info: Vec<ModelEntry> = Vec::new();
    let mut all_models_info: Vec<ModelEntry> = Vec::new();

    if let Some(payload) = payload {
        ingest_candidates(
            payload,
            url,
            &mut discovered_metadata,
            &mut models_info,
            &mut all_models_info,
        );
    }

    // 1577: an engine that lists models but marks none of them downloaded still gets a catalog, from
    // the list that kept the catalog-only entries.
    let collected = if models_info.is_empty() {
        &all_models_info
    } else {
        &models_info
    };
    let source_models = filter_embedding_model_entries(collected, &discovered_metadata);
    info!(
        "Model discovery: collected {} downloaded entries and {} catalog entries",
        models_info.len(),
        all_models_info.len()
    );

    let ordered = ordered_model_keys(&source_models);

    if !ordered.is_empty() {
        catalog.apply_known_models(&ordered, discovered_metadata);
        info!(
            "Discovered {} models via API (ordered by size asc): {:?}",
            ordered.len(),
            ordered
        );
        persist_model_discovery_cache(storage_root, &ordered, &catalog.metadata);
        return catalog.known_models.clone();
    }

    apply_discovery_fallback(
        catalog,
        previous_known_models,
        previous_metadata,
        storage_root,
        models_root,
    )
}

/// 1609-1629: the four ways out of a discovery that produced nothing, in the order the source tries
/// them. Each one says, in its own warning line, which of them it took.
fn apply_discovery_fallback(
    catalog: &mut ModelCatalog,
    previous_known_models: Vec<String>,
    previous_metadata: BTreeMap<String, ModelMetadata>,
    storage_root: &Path,
    models_root: Option<&Path>,
) -> Vec<String> {
    if !previous_known_models.is_empty() {
        catalog.apply_known_models(&previous_known_models, previous_metadata);
        warn!(
            "Model discovery returned no models; preserving {} existing models in memory",
            previous_known_models.len()
        );
        return catalog.known_models.clone();
    }

    let (cached_models, cached_metadata) = load_cached_model_discovery(storage_root);
    if !cached_models.is_empty() {
        catalog.apply_known_models(&cached_models, cached_metadata);
        warn!(
            "Model discovery returned no models; restored {} cached models",
            cached_models.len()
        );
        return catalog.known_models.clone();
    }

    let (disk_models, disk_metadata) = discover_models_from_disk(models_root);
    if !disk_models.is_empty() {
        catalog.apply_known_models(&disk_models, disk_metadata);
        warn!(
            "Model discovery returned no models; recovered {} models from disk",
            disk_models.len()
        );
        persist_model_discovery_cache(storage_root, &disk_models, &catalog.metadata);
        return catalog.known_models.clone();
    }

    catalog.apply_known_models(&[], BTreeMap::new());
    warn!("Model discovery returned no models and no fallback source was available");
    catalog.known_models.clone()
}

/// 1583-1594: sort smallest first, then take each model key once, in that order.
///
/// The sort is inside `if source_models:` in the source (1585), and `sort_by` on an empty `Vec` does
/// nothing anyway. It is a stable sort, so two models with the same size key keep the order the
/// payload listed them in.
///
/// The `else` branch at 1596-1600 walks `model_names`, which no path in `agent.py` ever fills:
/// `model_names = []` at 1543 is the only assignment to it, and the `if model_names: break` at 1574
/// can therefore never fire. It is not ported; see `docs/PARITY.md`, "Surfaces the clone does not
/// carry".
fn ordered_model_keys(source_models: &[ModelEntry]) -> Vec<String> {
    let mut sorted = source_models.to_vec();
    sorted.sort_by(|a, b| {
        size_sort_key(a)
            .partial_cmp(&size_sort_key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut ordered = Vec::new();
    let mut seen = Vec::new();
    for entry in &sorted {
        if entry.key.is_empty() || seen.iter().any(|seen: &String| seen == &entry.key) {
            continue;
        }
        seen.push(entry.key.clone());
        ordered.push(entry.key.clone());
    }
    ordered
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn candidates(text: &str) -> Vec<Value> {
        let payload: Value = serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("the fixture must be JSON: {error}"));
        parse_discovery_candidates(&payload)
    }

    #[test]
    fn a_models_key_is_the_first_shape_tried() {
        assert_eq!(
            candidates(r#"{"models": ["a", "b"]}"#),
            vec![json!("a"), json!("b")]
        );
    }

    #[test]
    fn a_bare_list_is_a_list() {
        assert_eq!(candidates(r#"[{"key": "a"}]"#), vec![json!({"key": "a"})]);
    }

    #[test]
    fn the_alternate_container_keys_are_tried_in_order() {
        for key in ["data", "available", "items", "list", "models"] {
            let payload = json!({ key: ["x"] });
            assert_eq!(
                parse_discovery_candidates(&payload),
                vec![json!("x")],
                "{key} must be read"
            );
        }
    }

    #[test]
    fn a_document_with_no_list_has_no_candidates() {
        assert!(candidates(r#"{"unexpected": "x"}"#).is_empty());
        assert!(candidates(r#""not a container""#).is_empty());
        assert!(candidates(r#"null"#).is_empty());
    }

    #[test]
    fn a_models_key_that_is_not_a_list_falls_through_to_the_other_keys() {
        // `{"models": "no", "data": ["x"]}`: 121 fails the isinstance test, so 127 finds `data`.
        assert_eq!(
            candidates(r#"{"models": "no", "data": ["x"]}"#),
            vec![json!("x")]
        );
    }

    #[test]
    fn the_largest_parameter_count_wins() {
        assert_eq!(parse_param_billions("9B"), Some(9.0));
        assert_eq!(parse_param_billions("30B-A3B"), Some(30.0));
        assert_eq!(parse_param_billions("64x2.6B"), Some(2.6));
        assert_eq!(parse_param_billions("7 b"), Some(7.0));
        assert_eq!(parse_param_billions("70b"), Some(70.0));
        assert_eq!(parse_param_billions("no parameters"), None);
    }

    #[test]
    fn a_numeric_params_key_is_not_read_as_a_parameter_count() {
        let payload = json!({"key": "a", "params": 30});
        assert_eq!(candidate_params(&payload), None);
        let payload = json!({"key": "a", "params_string": "30B"});
        assert_eq!(candidate_params(&payload), Some(30.0));
    }

    #[test]
    fn a_bare_string_is_a_downloaded_model_with_nothing_else_known() {
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &json!("  a  "),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );

        assert_eq!(models_info.len(), 1);
        assert_eq!(models_info[0].key, "a");
        assert_eq!(models_info[0].display, "a");
        assert_eq!(models_info[0].size, 0);
        // `downloaded` is not on the record: 1385 starts a bare string as downloaded, 1481 writes
        // that into the `setdefault`, and 1514 then replaces the whole record with four keys.
        assert_eq!(metadata["a"].downloaded, None);
        assert_eq!(metadata["a"].display, "a");
    }

    #[test]
    fn a_name_is_taken_from_the_first_key_that_has_one() {
        // Asserted on `all_models_info`, which every named entry reaches: none of these fixtures is
        // marked downloaded, so 1493 returns before the chat list.
        let payload =
            json!({"identifier": "third", "model": "fourth", "name": "fifth", "llm": "sixth"});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(all_models_info[0].key, "third");

        let payload = json!({"llm": "only"});
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(all_models_info[1].key, "only");
    }

    #[test]
    fn an_entry_with_no_name_is_ignored() {
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &json!({"display_name": "nameless"}),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        ingest_discovery_item(
            &json!(7),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert!(models_info.is_empty());
        assert!(all_models_info.is_empty());
        assert!(metadata.is_empty());
    }

    #[test]
    fn a_catalog_only_model_is_recorded_but_not_offered_for_chat() {
        let payload = json!({"key": "catalog-only", "state": "available"});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );

        assert!(models_info.is_empty());
        assert_eq!(
            all_models_info.len(),
            1,
            "the full catalog list still holds it"
        );
        assert_eq!(metadata["catalog-only"].downloaded, Some(false));
    }

    #[test]
    fn a_state_or_a_variant_list_can_only_make_a_model_downloaded() {
        let payload = json!({"key": "a", "downloaded": false, "state": "loaded"});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(models_info.len(), 1);

        let payload = json!({"key": "b", "downloadedVariants": ["q4_0"]});
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(models_info.len(), 2);
    }

    #[test]
    fn a_quantization_is_appended_to_the_display_name() {
        let payload = json!({"key": "a", "display_name": "Qwen", "quantization": {"name": "q4_0"}});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].display, "Qwen (q4_0)");

        let payload = json!({"key": "b", "display_name": "Qwen", "quantizationName": "q8_0"});
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["b"].display, "Qwen (q8_0)");

        // A key that contains a literal dot, which is what `item.get("quantization.name")` means.
        let payload = json!({"key": "c", "display_name": "Qwen", "quantization.name": "iq4"});
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["c"].display, "Qwen (iq4)");
    }

    #[test]
    fn the_first_entry_for_a_catalog_only_key_keeps_its_record() {
        // 1476's `setdefault`, and 1493 returning before any rewrite: a model nothing claims is
        // downloaded keeps the record the first entry wrote.
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &json!({"key": "a", "display_name": "First", "context_length": 4096}),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        ingest_discovery_item(
            &json!({"key": "a", "display_name": "Second", "context_length": 8192}),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].display, "First");
        assert_eq!(metadata["a"].context_length, Some(4096));
        assert_eq!(
            all_models_info.len(),
            2,
            "the full list still holds both entries"
        );
    }

    #[test]
    fn the_last_entry_for_a_downloaded_key_rewrites_its_record() {
        // The other half of the same pair: 1514 assigns rather than merges, so for a downloaded
        // chat model the last entry to arrive is the one the app keeps.
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &json!({"key": "a", "display_name": "First", "state": "loaded", "context_length": 4096}),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        ingest_discovery_item(
            &json!({"key": "a", "display_name": "Second", "state": "loaded", "context_length": 8192}),
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].display, "Second");
        assert_eq!(metadata["a"].context_length, None);
    }

    #[test]
    fn a_downloaded_chat_model_loses_the_fields_the_source_drops() {
        // 1514-1519. The context length was computed two lines earlier and is not kept, which is
        // why Stage 3's context-window lookup sees what the Python app sees.
        let payload = json!({"key": "a", "state": "loaded", "context_length": 8192, "params_string": "7B", "type": "llm"});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );

        let record = &metadata["a"];
        assert_eq!(record.display, "a");
        assert_eq!(record.context_length, None);
        assert_eq!(record.params, None);
        assert_eq!(record.model_type, None);
        assert_eq!(record.downloaded, None);
        assert_eq!(models_info[0].params, Some(7.0));
        assert_eq!(models_info[0].model_type.as_deref(), Some("llm"));
    }

    #[test]
    fn an_embedding_model_keeps_its_type_and_leaves_the_chat_list() {
        let payload =
            json!({"key": "embed", "state": "loaded", "type": "Embedding", "context_length": 512});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );

        assert!(models_info.is_empty());
        assert_eq!(all_models_info.len(), 1);
        assert_eq!(metadata["embed"].model_type.as_deref(), Some("embedding"));
        assert_eq!(
            metadata["embed"].context_length,
            Some(512),
            "the embedding branch updates the record rather than replacing it"
        );
    }

    #[test]
    fn a_selected_variant_is_its_own_load_target() {
        let payload = json!({"key": "a", "state": "loaded", "selected_variant": "q4_0", "variants": ["q4_0", "q8_0"]});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );

        assert_eq!(metadata["a"].variants, ["q4_0", "q8_0"]);
        assert_eq!(metadata["a"].selected_variant.as_deref(), Some("q4_0"));
        let keys: Vec<&str> = models_info.iter().map(|entry| entry.key.as_str()).collect();
        assert_eq!(keys, ["a", "q4_0"]);
        assert_eq!(
            models_info[1].model_type, None,
            "the variant entry has no type key"
        );

        let full: Vec<&str> = all_models_info
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(
            full,
            ["a", "q4_0", "a"],
            "1490 and 1527 both append the model itself"
        );
    }

    #[test]
    fn a_context_length_is_found_under_config_or_model() {
        // Catalog-only fixtures throughout this group: a downloaded chat model's record is replaced
        // at 1514-1519, which would drop the very field these tests are about.
        let payload = json!({"key": "a", "config": {"n_ctx": 32768}});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].context_length, Some(32768));

        let payload = json!({"key": "b", "model": {"context_window": 2048}});
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["b"].context_length, Some(2048));
    }

    #[test]
    fn a_context_length_that_is_not_a_number_is_no_context_length() {
        let payload = json!({"key": "a", "n_ctx": "wide"});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].context_length, None);
    }

    #[test]
    fn a_top_level_key_that_is_not_a_number_leaves_the_search_open() {
        // 1457-1461: `int("wide")` raises, `ctx` stays None, and the nested parent is consulted.
        let payload = json!({"key": "a", "n_ctx": "wide", "config": {"n_ctx": 4096}});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].context_length, Some(4096));
    }

    #[test]
    fn a_context_length_of_zero_is_no_context_length_and_it_ends_the_search() {
        // `int(0 or 0)` succeeds, so 1459 breaks out of the key loop and the nested `config` is
        // never read; and 1482 records 0 as None.
        let payload = json!({"key": "a", "context_length": 0, "config": {"n_ctx": 4096}});
        let mut metadata = BTreeMap::new();
        let mut models_info = Vec::new();
        let mut all_models_info = Vec::new();
        ingest_discovery_item(
            &payload,
            &mut metadata,
            &mut models_info,
            &mut all_models_info,
        );
        assert_eq!(metadata["a"].context_length, None);
    }

    /// A storage root and a models directory of its own under the approved temporary tree, so no
    /// test touches the operator's real `STORAGE_ROOT` or the real models folder.
    fn scratch(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir()
            .join("shed-models-discovery-tests")
            .join(label);
        let _ = std::fs::remove_dir_all(&root);
        let storage = root.join("storage");
        let models = root.join("models");
        std::fs::create_dir_all(&storage)
            .unwrap_or_else(|error| panic!("creating {storage:?}: {error}"));
        std::fs::create_dir_all(&models)
            .unwrap_or_else(|error| panic!("creating {models:?}: {error}"));
        (storage, models)
    }

    fn payload(text: &str) -> Value {
        serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("the fixture must be JSON: {error}"))
    }

    #[test]
    fn discovered_models_are_ordered_smallest_first_and_written_to_the_cache() {
        let (storage, models) = scratch("discovered");
        let mut catalog = ModelCatalog::default();
        let answer = payload(
            r#"{"models": [{"key": "big-70b", "state": "loaded"}, {"key": "small-8b", "state": "loaded"}]}"#,
        );

        let known = apply_discovered_payload(
            &mut catalog,
            Some(&answer),
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["small-8b", "big-70b"]);
        assert_eq!(catalog.known_models, known);
        let cached = std::fs::read_to_string(storage.join(crate::cache::CACHE_FILE_NAME))
            .unwrap_or_else(|error| panic!("reading the cache: {error}"));
        assert!(cached.contains("\"small-8b\""));
    }

    #[test]
    fn the_operator_s_engine_shape_reaches_the_catalog_through_the_full_list_only() {
        // What `GET http://127.0.0.1:8080/v1/models` actually answers on this machine: the list
        // under `data`, the name under `id`, and a `status.value` of `loaded` that
        // `ingest_discovery_item` never reads — it looks for `state`. So the model is catalog-only,
        // and it survives only because 1577 falls back to `all_models_info`.
        let (storage, models) = scratch("live-shape");
        let mut catalog = ModelCatalog::default();
        let answer = payload(
            r#"{"object": "list", "data": [{"id": "qwen3.8-flash-next-iq3_xxs", "object": "model",
                "status": {"value": "loaded"}, "meta": {"n_ctx": 204800}}]}"#,
        );

        let known = apply_discovered_payload(
            &mut catalog,
            Some(&answer),
            "http://127.0.0.1:8080/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["qwen3.8-flash-next-iq3_xxs"]);
        let record = &catalog.metadata["qwen3.8-flash-next-iq3_xxs"];
        assert_eq!(record.downloaded, Some(false));
        assert_eq!(
            record.context_length, None,
            "meta.n_ctx is not one of the keys the source reads"
        );
        assert_eq!(record.size, 0);
        assert_eq!(record.params, None);
    }

    #[test]
    fn an_empty_answer_keeps_the_models_the_app_already_believed() {
        let (storage, models) = scratch("previous");
        let mut catalog = ModelCatalog::default();
        catalog.apply_known_models(&["known-model".to_string()], BTreeMap::new());

        let known = apply_discovered_payload(
            &mut catalog,
            None,
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["known-model"]);
        assert!(
            !storage.join(crate::cache::CACHE_FILE_NAME).exists(),
            "the in-memory fallback does not write a cache"
        );
    }

    #[test]
    fn an_empty_answer_with_nothing_in_memory_reads_the_cache() {
        let (storage, models) = scratch("cache-fallback");
        std::fs::write(
            storage.join(crate::cache::CACHE_FILE_NAME),
            r#"{"known_models": ["cached-model"], "model_metadata": {"cached-model": {"display": "Cached"}}}"#,
        )
        .unwrap_or_else(|error| panic!("writing the cache: {error}"));

        let mut catalog = ModelCatalog::default();
        let known = apply_discovered_payload(
            &mut catalog,
            Some(&payload(r#"{"models": []}"#)),
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["cached-model"]);
        assert_eq!(catalog.metadata["cached-model"].display, "Cached");
    }

    #[test]
    fn an_empty_answer_with_no_cache_scans_the_models_directory_and_caches_it() {
        let (storage, models) = scratch("disk-fallback");
        std::fs::create_dir_all(models.join("Qwen").join("Qwen3.6-35B-A3B-FP8"))
            .unwrap_or_else(|error| panic!("creating the fixture: {error}"));

        let mut catalog = ModelCatalog::default();
        let known = apply_discovered_payload(
            &mut catalog,
            None,
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["Qwen/Qwen3.6-35B-A3B-FP8"]);
        assert!(
            storage.join(crate::cache::CACHE_FILE_NAME).exists(),
            "the disk fallback does write a cache (1624)"
        );
    }

    #[test]
    fn nothing_answered_and_nothing_to_fall_back_to_leaves_an_empty_catalog() {
        // Every fallback is empty in this fixture: nothing in memory, no cache file, and a models
        // directory with no `owner/model` pair in it.
        let (storage, models) = scratch("nothing");
        let mut catalog = ModelCatalog::default();

        let known = apply_discovered_payload(
            &mut catalog,
            None,
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert!(known.is_empty());
        assert!(catalog.known_models.is_empty());
        assert!(catalog.metadata.is_empty());
    }

    #[test]
    fn an_embedding_model_is_a_catalog_entry_but_not_a_chat_model() {
        let (storage, models) = scratch("embedding");
        let mut catalog = ModelCatalog::default();
        let answer = payload(
            r#"{"models": [{"key": "embed", "state": "loaded", "type": "embedding"}, {"key": "chat", "state": "loaded"}]}"#,
        );

        let known = apply_discovered_payload(
            &mut catalog,
            Some(&answer),
            "http://localhost:1234/v1/models",
            &storage,
            Some(&models),
        );

        assert_eq!(known, ["chat"]);
        assert!(
            catalog.metadata.contains_key("embed"),
            "1500-1511 keeps it in the metadata"
        );
    }
}
