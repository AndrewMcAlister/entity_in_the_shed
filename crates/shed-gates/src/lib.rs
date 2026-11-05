//! Workspace-wide invariant gates (How_To_Code principles 11, 16, 19, 23).
//!
//! Dev-only crate: no shipped crate may depend on it, and the layering gate
//! enforces that. Each gate module carries its own `#[cfg(test)]` tests, so
//! `cargo test --workspace` runs every gate; a violation fails the build with
//! the full list, not just the first.
//!
//! Gates that exist today: frozen assets ([`assets`]), crate layering
//! ([`layers`]), the env-read rule ([`env_read`]). The settings-template and
//! guard-catalogue gates arrive with Stage 1 and with the first guard.

pub mod assets;
pub mod env_read;
pub mod layers;

use std::path::{Path, PathBuf};

/// Repository root, derived at compile time from this crate's location
/// (`crates/shed-gates` -> repo root). Compile-time on purpose: reading the
/// environment at runtime would trip the env-read gate this crate enforces.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Relative path from the repo root with `/` separators, matching the
/// manifest's and the layer map's path style on every platform.
pub fn relative_to_root(root: &Path, path: &Path) -> String {
    let rel = match path.strip_prefix(root) {
        Ok(rel) => rel,
        Err(_) => path,
    };
    rel.to_string_lossy().replace('\\', "/")
}

/// Sorted immediate child directories of `parent` (empty when it is absent,
/// e.g. `bins/` before Stage 1 creates the binary).
pub fn child_dirs(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries.filter_map(Result::ok).collect();
    names.sort_by_key(std::fs::DirEntry::file_name);
    names
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// Every file under `dir`, depth-first, sorted at each level so gate output
/// and traversal are deterministic.
pub fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut names: Vec<_> = entries.filter_map(Result::ok).collect();
    names.sort_by_key(std::fs::DirEntry::file_name);
    for entry in names {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else if path.is_file() {
            out.push(path);
        }
    }
    out
}
