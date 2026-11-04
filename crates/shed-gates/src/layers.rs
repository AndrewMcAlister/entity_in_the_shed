//! Plan 3.3: the crate dependency direction is fixed, has no cycles, and only `shed-config`
//! reads the environment (`How_To_Code` principles 19 and 24).
//!
//! The rules live here as data so a violation names the crate and the edge instead of
//! "layering broke". Cargo does not check dependency direction and clippy cannot express
//! it, so these tests are the gate.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The crates any composition-layer package may reach. `shed-http` is the composition root
/// apart from the binary, so its list is exactly this.
pub const DOMAIN: [&str; 16] = [
    "shed-config",
    "shed-json",
    "shed-text",
    "shed-store",
    "shed-logging",
    "shed-models",
    "shed-prompts",
    "shed-characters",
    "shed-llm",
    "shed-conversation",
    "shed-tools",
    "shed-mcp",
    "shed-chat",
    "shed-tts",
    "shed-entity",
    "shed-mind",
];

/// Every package in the workspace and the crates each one may depend on. An edge is allowed
/// only if it appears here.
pub const LAYERS: [(&str, &[&str]); 20] = [
    ("shed-config", &[]),
    ("shed-json", &["shed-config"]),
    ("shed-text", &["shed-config"]),
    ("shed-store", &["shed-config", "shed-json"]),
    ("shed-logging", &["shed-config"]),
    ("shed-models", &["shed-config", "shed-json", "shed-text", "shed-store"]),
    ("shed-prompts", &["shed-config", "shed-json", "shed-text", "shed-store"]),
    ("shed-characters", &["shed-config", "shed-json", "shed-text", "shed-store"]),
    ("shed-llm", &["shed-config", "shed-json", "shed-models", "shed-prompts", "shed-text"]),
    (
        "shed-conversation",
        &["shed-config", "shed-json", "shed-text", "shed-store", "shed-models", "shed-prompts"],
    ),
    ("shed-tools", &["shed-config", "shed-json", "shed-text", "shed-store"]),
    ("shed-mcp", &["shed-config", "shed-json", "shed-text", "shed-store"]),
    (
        "shed-chat",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-models",
            "shed-llm",
            "shed-prompts",
            "shed-tools",
            "shed-mcp",
            "shed-conversation",
        ],
    ),
    ("shed-tts", &["shed-config", "shed-text", "shed-store"]),
    ("shed-entity", &["shed-config", "shed-json", "shed-store"]),
    (
        "shed-mind",
        &[
            "shed-entity",
            "shed-models",
            "shed-llm",
            "shed-prompts",
            "shed-conversation",
            "shed-tools",
        ],
    ),
    ("shed-http", &DOMAIN),
    (
        "shed-server",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-store",
            "shed-logging",
            "shed-models",
            "shed-prompts",
            "shed-characters",
            "shed-llm",
            "shed-conversation",
            "shed-tools",
            "shed-mcp",
            "shed-chat",
            "shed-tts",
            "shed-entity",
            "shed-mind",
            "shed-http",
        ],
    ),
    ("shed-testutil", &DOMAIN),
    ("shed-gates", &["shed-config", "shed-testutil"]),
];

/// Packages that hold test helpers only. No shipped crate may depend on them at runtime.
pub const TEST_ONLY_CRATES: [&str; 2] = ["shed-testutil", "shed-gates"];

/// Principle 19 and principle 24: the environment is read once, in `shed-config`, and every
/// other crate is handed an `AppConfig`. `env!(...)` is a compile-time macro, not a read, so
/// it is not matched here.
pub const ENV_READ_MARKERS: [&str; 4] = ["env::var(", "env::vars(", "env::var_os(", "dotenvy::"];

/// The line with the inside of every string literal blanked out.
///
/// The scan below looks for [`ENV_READ_MARKERS`], and this file contains those markers as
/// text, including in the fixtures of its own tests. Blanking literals keeps those from
/// matching themselves while a real read, `env::var("MODEL_PROVIDER")`, still reads as
/// `env::var("")` and is caught. Char literals are left alone: one holds a single character
/// and every marker is longer than that, so a marker cannot hide inside one.
pub fn code_without_literals(line: &str) -> String {
    let mut code = String::with_capacity(line.len());
    let mut rest = line;

    while !rest.is_empty() {
        let Some((at, ch)) = rest.char_indices().find(|(_, ch)| *ch == '"' || *ch == 'r') else {
            code.push_str(rest);
            return code;
        };
        code.push_str(&rest[..at]);
        let tail = &rest[at + ch.len_utf8()..];

        match ch {
            '"' => rest = quoted_tail(tail),
            'r' => match raw_tail(tail) {
                Some(after) => rest = after,
                None => {
                    code.push('r');
                    rest = tail;
                }
            },
            _ => unreachable!("only \" and r are searched for above"),
        }
    }
    code
}

/// After an opening `"`, the text following the closing quote. An ordinary Rust string
/// cannot span lines, so an unterminated one means there is nothing left on this line.
fn quoted_tail(tail: &str) -> &str {
    let mut chars = tail.char_indices();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '"' => return &tail[at + 1..],
            _ => {}
        }
    }
    &tail[tail.len()..]
}

/// After an `r`, the text following a raw literal (`r"`, `r#"`, `r##"` ...), or `None` when
/// the `r` is ordinary code rather than the start of a raw literal.
fn raw_tail(tail: &str) -> Option<&str> {
    let hashes = tail.chars().take_while(|ch| *ch == '#').count();
    let body = tail.get(hashes..)?.strip_prefix('"')?;
    let terminator = format!("\"{}", "#".repeat(hashes));
    match body.find(terminator.as_str()) {
        Some(at) => Some(&body[at + terminator.len()..]),
        None => Some(""),
    }
}

/// The crates a package may depend on, or `None` when the package is not in the table.
pub fn allowed_for(name: &str) -> Option<&'static [&'static str]> {
    LAYERS.iter().find(|(layer, _)| *layer == name).map(|(_, allowed)| *allowed)
}

/// The `Cargo.toml` of every package in this workspace.
pub fn package_manifests(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut manifests = Vec::new();
    for group in ["crates", "bins"] {
        let dir = root.join(group);
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let manifest = entry?.path().join("Cargo.toml");
            if manifest.is_file() {
                manifests.push(manifest);
            }
        }
    }
    manifests.sort();
    Ok(manifests)
}

/// The `name = "..."` line of a package manifest.
pub fn package_name(manifest: &Path) -> Result<String, Box<dyn std::error::Error>> {
    for line in fs::read_to_string(manifest)?.lines() {
        if let Some(value) = line.trim().strip_prefix("name = ") {
            return Ok(value.trim().trim_matches('"').to_string());
        }
    }
    Err(Box::from(format!("{} has no package name", manifest.display())))
}

/// The `.rs` files a package owns, in `src/` and `tests/`.
pub fn package_sources(package_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ["src", "tests"] {
        collect_rs(&package_dir.join(dir), &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn collect_rs(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rs(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// The `shed-*` packages a manifest depends on, split into normal and dev dependencies.
pub fn shed_dependencies(manifest: &Path) -> io::Result<(Vec<String>, Vec<String>)> {
    let mut normal = Vec::new();
    let mut dev = Vec::new();
    let mut section = String::new();

    for raw in fs::read_to_string(manifest)?.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        if line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let name = line.split('=').next().unwrap_or_default().trim().to_string();
        if !name.starts_with("shed-") {
            continue;
        }
        match section.as_str() {
            "dependencies" => normal.push(name),
            "dev-dependencies" => dev.push(name),
            _ => {}
        }
    }
    normal.sort();
    dev.sort();
    Ok((normal, dev))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gate, repo_root};

    #[test]
    fn every_workspace_package_is_declared_in_the_layer_table() -> Gate {
        let mut declared = Vec::new();
        for manifest in package_manifests(&repo_root())? {
            declared.push(package_name(&manifest)?);
        }

        let mut problems = Vec::new();
        for (layer, _) in LAYERS {
            if !declared.iter().any(|name| name == layer) {
                problems.push(format!("{layer} is in the layer table but has no package"));
            }
        }
        for name in &declared {
            if allowed_for(name).is_none() {
                problems.push(format!("{name} is not in the layer table in docs/ARCHITECTURE.md"));
            }
        }

        if !problems.is_empty() {
            return Err(Box::from(format!(
                "the workspace and the layer table disagree:\n{}",
                problems.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn no_crate_depends_outside_its_layer() -> Gate {
        let mut problems = Vec::new();

        for manifest in package_manifests(&repo_root())? {
            let name = package_name(&manifest)?;
            let Some(allowed) = allowed_for(&name) else {
                continue;
            };
            let (normal, dev) = shed_dependencies(&manifest)?;

            for edge in &normal {
                if TEST_ONLY_CRATES.contains(&edge.as_str()) {
                    problems.push(format!(
                        "{name} -> {edge}: a test-only package may not be a runtime dependency"
                    ));
                } else if !allowed.contains(&edge.as_str()) {
                    problems.push(format!("{name} -> {edge}: not allowed by the layer table"));
                }
            }
            for edge in &dev {
                if edge == &name {
                    problems.push(format!("{name} dev-depends on itself"));
                } else if !allowed.contains(&edge.as_str()) {
                    problems
                        .push(format!("{name} -> {edge} (dev): not allowed by the layer table"));
                }
            }
        }

        if !problems.is_empty() {
            return Err(Box::from(format!(
                "crate dependency direction is broken (plan 3.3):\n{}",
                problems.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn only_shed_config_reads_the_environment() -> Gate {
        let mut offenders = Vec::new();

        for manifest in package_manifests(&repo_root())? {
            let name = package_name(&manifest)?;
            let Some(package_dir) = manifest.parent() else {
                continue;
            };
            if name == "shed-config" {
                continue;
            }
            for path in package_sources(package_dir)? {
                let code = fs::read_to_string(&path)?;
                for (number, line) in code.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    let stripped = code_without_literals(line);
                    if ENV_READ_MARKERS.iter().any(|marker| stripped.contains(marker)) {
                        offenders.push(format!(
                            "{}:{} reads the environment",
                            path.display(),
                            number + 1
                        ));
                    }
                }
            }
        }

        if !offenders.is_empty() {
            return Err(Box::from(format!(
                "only shed-config may read the environment (principles 19 and 24):\n{}",
                offenders.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn the_env_scan_catches_a_read_and_ignores_the_marker_table() {
        let read = r#"let provider = env::var("MODEL_PROVIDER").unwrap_or_default();"#;
        let table = r#"["env::var(", "env::vars(", "env::var_os(", "dotenvy::"];"#;
        let lifetime = "pub fn allowed_for(name: &str) -> Option<&'static [&'static str]> {";

        let read_is_a_read =
            ENV_READ_MARKERS.iter().any(|marker| code_without_literals(read).contains(marker));
        let table_is_not =
            ENV_READ_MARKERS.iter().any(|marker| code_without_literals(table).contains(marker));

        assert!(read_is_a_read, "a real env::var call must be caught");
        assert!(!table_is_not, "the marker table must not match itself");
        assert_eq!(
            code_without_literals(lifetime),
            lifetime,
            "lifetimes must survive the scan unchanged"
        );
    }
}
