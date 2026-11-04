//! Decision D2: the browser client, the data it reads and the eight browser suites are
//! copied from the Python app byte-for-byte and frozen. The Rust server has to satisfy the
//! client that already exists, so the client is not allowed to move under it.
//!
//! `tests/assets.manifest.json` records the path, byte count and SHA-256 of every frozen
//! file. The tests below fail on three kinds of drift: a frozen file whose bytes changed,
//! a manifest entry that is missing, and a new file that appeared inside a frozen tree.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Trees in which every file is frozen, and the single frozen files. The names mirror the
/// Python app because the eight browser suites reach the client through them:
/// `tests/test_*.js` does `require('../static/app-helpers.js')`.
pub const FROZEN_DIRS: [&str; 6] =
    ["templates", "static", "styles", "characters", "prompts", "outputs"];
pub const FROZEN_FILES: [&str; 1] = ["voices.json"];
pub const FROZEN_MANIFEST: &str = "tests/assets.manifest.json";

/// One entry of `tests/assets.manifest.json`.
#[derive(Debug)]
pub struct FrozenFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// The frozen manifest, keyed by repo-relative path.
pub type Manifest = BTreeMap<String, FrozenFile>;

/// Every frozen file on disk, keyed by repo-relative path with `/` separators.
pub fn frozen_files_on_disk(root: &Path) -> io::Result<BTreeMap<String, PathBuf>> {
    let mut found = BTreeMap::new();
    for dir in FROZEN_DIRS {
        collect_files(&root.join(dir), root, &mut found)?;
    }
    for file in FROZEN_FILES {
        let path = root.join(file);
        if path.is_file() {
            found.insert(file.to_string(), path);
        }
    }
    for path in js_suites(&root.join("tests"))? {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        found.insert(format!("tests/{name}"), path);
    }
    Ok(found)
}

/// The unchanged browser suites, kept at the repository's `tests/` directory so that their
/// `require('../static/app-helpers.js')` resolves without editing them.
pub fn js_suites(tests_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut suites = Vec::new();
    if !tests_dir.is_dir() {
        return Ok(suites);
    }
    for entry in fs::read_dir(tests_dir)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.starts_with("test_") && name.ends_with(".js") {
            suites.push(path);
        }
    }
    suites.sort();
    Ok(suites)
}

pub fn frozen_manifest(root: &Path) -> Result<Manifest, Box<dyn std::error::Error>> {
    let manifest_path = root.join(FROZEN_MANIFEST);
    let text = fs::read_to_string(&manifest_path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let entries = value["files"]
        .as_array()
        .ok_or_else(|| format!("{} has no \"files\" array", manifest_path.display()))?;

    let mut parsed = Manifest::new();
    for entry in entries {
        let Some(path) = entry["path"].as_str() else {
            return Err(Box::from(format!("{} has an entry without a path", FROZEN_MANIFEST)));
        };
        let Some(bytes) = entry["bytes"].as_u64() else {
            return Err(Box::from(format!("{path} has no byte count")));
        };
        let Some(sha256) = entry["sha256"].as_str() else {
            return Err(Box::from(format!("{path} has no sha256")));
        };
        parsed.insert(
            path.to_string(),
            FrozenFile { path: path.to_string(), bytes, sha256: sha256.to_string() },
        );
    }
    Ok(parsed)
}

/// Every file under `dir`, keyed relative to `root`.
pub fn collect_files(
    dir: &Path,
    root: &Path,
    found: &mut BTreeMap<String, PathBuf>,
) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, root, found)?;
        } else {
            found.insert(relative_path(&path, root)?, path);
        }
    }
    Ok(())
}

pub fn relative_path(path: &Path, root: &Path) -> io::Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| io::Error::other(format!("{path:?} is outside {root:?}")))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

pub fn sha256_of(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path)?);
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gate, repo_root};

    #[test]
    fn every_manifest_entry_is_on_disk_and_unchanged() -> Gate {
        let root = repo_root();
        let on_disk = frozen_files_on_disk(&root)?;
        let mut problems = Vec::new();

        for (path, expected) in frozen_manifest(&root)? {
            let Some(actual) = on_disk.get(path.as_str()) else {
                problems.push(format!("{path}: listed in the manifest but not on disk"));
                continue;
            };
            let bytes = fs::metadata(actual)?.len();
            if bytes != expected.bytes {
                problems.push(format!(
                    "{path}: {bytes} bytes on disk, manifest says {}",
                    expected.bytes
                ));
            }
            let digest = sha256_of(actual)?;
            if digest != expected.sha256 {
                problems
                    .push(format!("{path}: sha256 {digest}, manifest says {}", expected.sha256));
            }
        }

        if !problems.is_empty() {
            return Err(Box::from(format!(
                "the frozen client or data has drifted (decision D2):\n{}",
                problems.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn nothing_extra_appears_in_a_frozen_tree() -> Gate {
        let root = repo_root();
        let manifest = frozen_manifest(&root)?;
        let on_disk = frozen_files_on_disk(&root)?;
        let extra: Vec<&String> =
            on_disk.keys().filter(|path| !manifest.contains_key(path.as_str())).collect();

        if !extra.is_empty() {
            return Err(Box::from(format!(
                "files exist in a frozen tree with no manifest entry (decision D2):\n{}",
                extra.iter().map(|path| path.as_str()).collect::<Vec<_>>().join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn the_eight_browser_suites_are_present_and_frozen() -> Gate {
        let root = repo_root();
        let manifest = frozen_manifest(&root)?;
        let suites = js_suites(&root.join("tests"))?;

        if suites.len() != 8 {
            return Err(Box::from(format!(
                "expected the 8 unchanged browser suites, found {}",
                suites.len()
            )));
        }
        for path in &suites {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !manifest.contains_key(format!("tests/{name}").as_str()) {
                return Err(Box::from(format!(
                    "tests/{name}: a browser suite with no manifest entry (decision D2)"
                )));
            }
        }
        Ok(())
    }
}
