//! Frozen-asset gate (decision D2).
//!
//! The browser client, the data it reads and the eight browser suites are
//! copied byte-for-byte from the Python app and are never edited: the Rust
//! server must satisfy the contract they already implement. This gate
//! compares every byte on disk under the frozen roots against
//! `tests/assets.manifest.json` (path + bytes + sha256), in both directions:
//! a listed file whose bytes drifted, and an unlisted file that appeared,
//! both fail.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Manifest {
    files: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}

/// Directories where every file must be listed in the manifest.
const FROZEN_DIRS: &[&str] = &[
    "templates",
    "static",
    "styles",
    "characters",
    "prompts",
    "outputs",
];

/// The manifest itself is not an asset; under `tests/` only the JS suites
/// are frozen.
const TESTS_DIR: &str = "tests";
const MANIFEST_PATH: &str = "tests/assets.manifest.json";

/// Every violation, empty when the frozen set is intact.
pub fn violations() -> Vec<String> {
    let root = crate::workspace_root();
    let text = match fs::read_to_string(root.join(MANIFEST_PATH)) {
        Ok(text) => text,
        Err(error) => return vec![format!("{MANIFEST_PATH}: unreadable ({error})")],
    };
    let manifest: Manifest = match serde_json::from_str(&text) {
        Ok(manifest) => manifest,
        Err(error) => return vec![format!("{MANIFEST_PATH}: invalid JSON ({error})")],
    };
    let mut out = Vec::new();
    let mut listed = BTreeSet::new();
    for entry in &manifest.files {
        listed.insert(entry.path.clone());
        check_listed(&root, entry, &mut out);
    }
    find_unlisted(&root, &listed, &mut out);
    out
}

fn check_listed(root: &Path, entry: &Entry, out: &mut Vec<String>) {
    let full = root.join(&entry.path);
    let Ok(bytes) = fs::read(&full) else {
        out.push(format!("{}: listed but unreadable", entry.path));
        return;
    };
    if bytes.len() as u64 != entry.bytes {
        out.push(format!(
            "{}: {} bytes on disk, manifest says {}",
            entry.path,
            bytes.len(),
            entry.bytes
        ));
    }
    let digest = hex_sha256(&bytes);
    if digest != entry.sha256 {
        out.push(format!(
            "{}: sha256 {digest}, manifest says {}",
            entry.path, entry.sha256
        ));
    }
}

fn find_unlisted(root: &Path, listed: &BTreeSet<String>, out: &mut Vec<String>) {
    for dir in FROZEN_DIRS {
        let full = root.join(dir);
        if !full.is_dir() {
            out.push(format!("{dir}/: frozen root missing"));
            continue;
        }
        for path in crate::walk_files(&full) {
            let rel = crate::relative_to_root(root, &path);
            if !listed.contains(&rel) {
                out.push(format!("{rel}: on disk but not in the manifest"));
            }
        }
    }
    let tests = root.join(TESTS_DIR);
    for path in crate::walk_files(&tests) {
        let rel = crate::relative_to_root(root, &path);
        let is_suite = rel.ends_with(".js");
        if is_suite && !listed.contains(&rel) {
            out.push(format!("{rel}: on disk but not in the manifest"));
        }
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn frozen_assets_match_manifest() {
        let found = super::violations();
        assert!(
            found.is_empty(),
            "frozen-asset gate violated (decision D2):\n{}",
            found.join("\n")
        );
    }
}
