//! The models directory on disk: the fallback that works when nothing answers the HTTP request.
//!
//! Ported from `agent.py:1327` (`discover_lm_studio_models_from_disk`). The directory it scans is
//! `lmstudio_models_root` (1312), which `shed-config` resolves into `LmStudio::models_dir`
//! (`crates/shed-config/src/lm.rs`); this function is handed the result and has no environment to
//! read.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tracing::info;

use crate::catalog::ModelMetadata;
use crate::size::parse_model_size_billions;

/// The catalog a directory scan can build: `owner/model` for every pair of directories under the
/// models root, smallest name first.
///
/// An engine's model list and a models directory are different lists. LM Studio names a model
/// `owner/model` on disk, and `ingest_discovery_item` names it whatever the API called it, which on
/// the operator's machine is bare `qwen3.8-flash-next-iq3_xxs`. A disk scan is therefore a real
/// fallback and not a second copy of the same catalog.
pub(crate) fn discover_models_from_disk(
    models_root: Option<&Path>,
) -> (Vec<String>, BTreeMap<String, ModelMetadata>) {
    let Some(root) = models_root.filter(|root| root.is_dir()) else {
        return (Vec::new(), BTreeMap::new());
    };

    let mut ordered: Vec<String> = Vec::new();
    let mut metadata = BTreeMap::new();

    for owner in child_directories(root) {
        let owner_name = file_name(&owner);
        for model in child_directories(&owner) {
            let model_key = format!("{owner_name}/{}", file_name(&model));
            let record = ModelMetadata {
                display: model_key.clone(),
                downloaded: Some(true),
                source: Some("disk".to_string()),
                ..ModelMetadata::default()
            };
            ordered.push(model_key.clone());
            metadata.insert(model_key, record);
        }
    }

    // 1349: `ordered.sort(key=parse_model_size_billions)`, a stable sort on the name alone, so two
    // models of unknown size keep the alphabetical order the directory listing produced.
    ordered.sort_by(|a, b| {
        size_of_name(a)
            .partial_cmp(&size_of_name(b))
            .unwrap_or(Ordering::Equal)
    });
    if !ordered.is_empty() {
        info!(
            "Recovered {} LM Studio models from disk scan",
            ordered.len()
        );
    }

    (ordered, metadata)
}

/// `sorted(iterdir(), key=lambda item: item.name.lower())` keeping only directories
/// (`agent.py:1334-1339`). A directory that cannot be listed contributes nothing: the source would
/// raise here and take the request with it, which the clone does not do.
fn child_directories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut children: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| (file_name(&path).to_lowercase(), path))
        .collect();
    children.sort_by(|a, b| a.0.cmp(&b.0));
    children.into_iter().map(|(_, path)| path).collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The sort value. `f64::cmp` is total — it orders NaN too — so `INFINITY` for "nothing measured"
/// needs no special handling beyond putting those models last.
fn size_of_name(name: &str) -> f64 {
    parse_model_size_billions(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join("shed-models-disk-tests")
            .join(label);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("creating {} must work: {error}", root.display()));
        root
    }

    fn make_model(root: &Path, owner: &str, model: &str) {
        let dir = root.join(owner).join(model);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|error| panic!("creating {} must work: {error}", dir.display()));
    }

    #[test]
    fn a_missing_or_absent_models_directory_finds_nothing() {
        let (models, metadata) = discover_models_from_disk(None);
        assert!(models.is_empty());
        assert!(metadata.is_empty());

        let missing = scratch_root("missing").join("no-such-directory");
        let (models, _) = discover_models_from_disk(Some(&missing));
        assert!(models.is_empty());
    }

    #[test]
    fn a_model_is_owner_slash_model() {
        let root = scratch_root("owner-model");
        make_model(&root, "Qwen", "Qwen3.6-35B-A3B-FP8");

        let (models, metadata) = discover_models_from_disk(Some(&root));
        assert_eq!(models, ["Qwen/Qwen3.6-35B-A3B-FP8"]);
        let record = &metadata["Qwen/Qwen3.6-35B-A3B-FP8"];
        assert_eq!(record.display, "Qwen/Qwen3.6-35B-A3B-FP8");
        assert_eq!(record.downloaded, Some(true));
        assert_eq!(record.source.as_deref(), Some("disk"));
        assert_eq!(record.context_length, None);
    }

    #[test]
    fn only_directories_count_and_the_names_sort_case_insensitively() {
        let root = scratch_root("sorting");
        make_model(&root, "bobs", "Alpha");
        make_model(&root, "Ashton", "zeta");
        std::fs::write(root.join("a-loose-file.txt"), "not a model")
            .unwrap_or_else(|error| panic!("writing the fixture: {error}"));

        let (models, _) = discover_models_from_disk(Some(&root));
        // Both names have no size hint, so the alphabetical order the listing produced stands.
        assert_eq!(models, ["Ashton/zeta", "bobs/Alpha"]);
    }

    #[test]
    fn the_size_written_in_a_directory_name_orders_the_scan() {
        let root = scratch_root("size-order");
        make_model(&root, "owner", "mistral-70b");
        make_model(&root, "owner", "llama-8b");
        make_model(&root, "owner", "unknown-name");

        let (models, _) = discover_models_from_disk(Some(&root));
        assert_eq!(
            models,
            ["owner/llama-8b", "owner/mistral-70b", "owner/unknown-name"]
        );
    }

    #[test]
    fn an_owner_directory_with_no_models_contributes_nothing() {
        let root = scratch_root("empty-owner");
        std::fs::create_dir_all(root.join("nobody"))
            .unwrap_or_else(|error| panic!("creating the fixture: {error}"));
        let (models, metadata) = discover_models_from_disk(Some(&root));
        assert!(models.is_empty());
        assert!(metadata.is_empty());
    }
}
