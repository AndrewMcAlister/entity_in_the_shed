//! Crate layering gate (decision D4, plan §3.3).
//!
//! [`LAYERS`] is the complete, forward-looking map: it names crates that do
//! not exist yet, and the gate checks whatever packages exist today against
//! it (crates are created on demand). Extra invariants ride along:
//! `shed-testutil` may only appear under `[dev-dependencies]` (of anyone),
//! `shed-gates` has no consumers at all (its tests run in its own package),
//! `shed-gates`' own internal dependencies must themselves be dev-dependencies,
//! and any `shed-*` package that exists must appear in the map.

use std::fs;
use std::path::Path;

/// Crate name -> the internal (`shed-*`) crates it may depend on, under any
/// dependency kind. External crates are not layer-checked here.
pub const LAYERS: &[(&str, &[&str])] = &[
    ("shed-config", &[]),
    ("shed-json", &["shed-config"]),
    ("shed-text", &["shed-config"]),
    ("shed-logging", &["shed-config"]),
    ("shed-store", &["shed-config", "shed-json"]),
    (
        "shed-models",
        &["shed-config", "shed-json", "shed-text", "shed-store"],
    ),
    (
        "shed-prompts",
        &["shed-config", "shed-json", "shed-text", "shed-store"],
    ),
    (
        "shed-characters",
        &["shed-config", "shed-json", "shed-text", "shed-store"],
    ),
    (
        "shed-llm",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-models",
            "shed-prompts",
        ],
    ),
    (
        "shed-conversation",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-store",
            "shed-models",
            "shed-prompts",
        ],
    ),
    (
        "shed-tools",
        &["shed-config", "shed-json", "shed-text", "shed-store"],
    ),
    (
        "shed-mcp",
        &["shed-config", "shed-json", "shed-text", "shed-store"],
    ),
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
            "shed-config",
            "shed-entity",
            "shed-models",
            "shed-llm",
            "shed-prompts",
            "shed-conversation",
            "shed-tools",
        ],
    ),
    (
        "shed-http",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-logging",
            "shed-store",
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
        ],
    ),
    (
        "shed-server",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-logging",
            "shed-store",
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
    (
        "shed-testutil",
        &[
            "shed-config",
            "shed-json",
            "shed-text",
            "shed-logging",
            "shed-store",
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
        ],
    ),
    ("shed-gates", &["shed-config", "shed-testutil"]),
];

/// Every violation against the layer map, empty when the existing packages obey it.
pub fn violations() -> Vec<String> {
    let root = crate::workspace_root();
    let mut out = Vec::new();
    for (name, allowed) in LAYERS {
        let manifest = root.join(package_rel_dir(name)).join("Cargo.toml");
        if manifest.is_file() {
            check_package(name, allowed, &manifest, &mut out);
        }
    }
    check_known_map(&root, &mut out);
    out
}

/// The binary lives under `bins/`, every library crate under `crates/`.
fn package_rel_dir(name: &str) -> String {
    if name == "shed-server" {
        format!("bins/{name}")
    } else {
        format!("crates/{name}")
    }
}

fn check_package(name: &str, allowed: &[&str], manifest: &Path, out: &mut Vec<String>) {
    let Ok(text) = fs::read_to_string(manifest) else {
        out.push(format!("{name}: Cargo.toml unreadable"));
        return;
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        out.push(format!("{name}: Cargo.toml does not parse"));
        return;
    };
    for (kind, dep) in dependency_entries(&doc) {
        if !dep.starts_with("shed-") {
            continue;
        }
        let is_dev = kind == "dev-dependencies";
        if dep == "shed-gates" {
            out.push(format!(
                "{name}: shed-gates has no consumers; its tests run in its own package"
            ));
        } else if dep == "shed-testutil" {
            if !is_dev {
                out.push(format!(
                    "{name}: shed-testutil is dev-only but sits under [{kind}]"
                ));
            }
        } else if !allowed.contains(&dep.as_str()) {
            out.push(format!("{name}: may not depend on {dep} (layer map)"));
        } else if name == "shed-gates" && !is_dev {
            out.push(format!("{name}: its internal dependencies are dev-dependencies only, found [{kind}] -> {dep}"));
        }
    }
}

/// (dependency-kind, dependency-name) pairs, including target-specific sections.
fn dependency_entries(doc: &toml::Table) -> Vec<(String, String)> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut out = Vec::new();
    for kind in KINDS {
        if let Some(table) = doc.get(kind).and_then(toml::Value::as_table) {
            out.extend(table.keys().map(|name| (kind.to_string(), name.clone())));
        }
    }
    if let Some(targets) = doc.get("target").and_then(toml::Value::as_table) {
        for body in targets.values().filter_map(toml::Value::as_table) {
            for kind in KINDS {
                if let Some(table) = body.get(kind).and_then(toml::Value::as_table) {
                    out.extend(table.keys().map(|name| (kind.to_string(), name.clone())));
                }
            }
        }
    }
    out
}

/// A `shed-*` package that exists but is not in the map means the map went stale.
fn check_known_map(root: &Path, out: &mut Vec<String>) {
    for parent in ["crates", "bins"] {
        for crate_dir in crate::child_dirs(&root.join(parent)) {
            let manifest = crate_dir.join("Cargo.toml");
            let Some(name) = package_name(&manifest) else {
                continue;
            };
            let known = LAYERS.iter().any(|(known, _)| *known == name);
            if name.starts_with("shed-") && !known {
                out.push(format!(
                    "{name}: package exists but is not in the layer map"
                ));
            }
        }
    }
}

fn package_name(manifest: &Path) -> Option<String> {
    let text = fs::read_to_string(manifest).ok()?;
    let doc = text.parse::<toml::Table>().ok()?;
    let name = doc.get("package")?.get("name")?.get("value")?.as_str()?;
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn existing_packages_obey_the_layer_map() {
        let found = super::violations();
        assert!(
            found.is_empty(),
            "layering gate violated:\n{}",
            found.join("\n")
        );
    }

    /// The scanner itself: a forbidden internal dependency is named; an
    /// allowed one and a dev-only `shed-testutil` are not.
    #[test]
    fn scanner_names_forbidden_dependencies() -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_cargo_toml(
            "scanner",
            "[package]\nname = \"shed-tts\"\n\
             [dependencies]\nshed-llm = { workspace = true }\nshed-text = { workspace = true }\n\
             [dev-dependencies]\nshed-testutil = { workspace = true }\n",
        )?;
        let mut out = Vec::new();
        super::check_package(
            "shed-tts",
            allowed_for("shed-tts"),
            &dir.join("Cargo.toml"),
            &mut out,
        );
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(
            out,
            vec!["shed-tts: may not depend on shed-llm (layer map)".to_owned()],
            "shed-text and the dev shed-testutil are legal for shed-tts; shed-llm is not"
        );
        Ok(())
    }

    /// `shed-gates` is never a consumer's dependency.
    #[test]
    fn scanner_rejects_gates_as_dependency() -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_cargo_toml(
            "gatesdep",
            "[package]\nname = \"shed-chat\"\n[dependencies]\nshed-gates = { path = \"../shed-gates\" }\n",
        )?;
        let mut out = Vec::new();
        super::check_package(
            "shed-chat",
            allowed_for("shed-chat"),
            &dir.join("Cargo.toml"),
            &mut out,
        );
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(
            out,
            vec![
                "shed-chat: shed-gates has no consumers; its tests run in its own package"
                    .to_owned()
            ]
        );
        Ok(())
    }

    /// `shed-gates`' own internal dependencies must be dev-dependencies.
    #[test]
    fn scanner_requires_gates_internal_deps_to_be_dev() -> Result<(), Box<dyn std::error::Error>> {
        let dir = temp_cargo_toml(
            "gatesdev",
            "[package]\nname = \"shed-gates\"\n[dependencies]\nshed-config = { workspace = true }\n",
        )?;
        let mut out = Vec::new();
        super::check_package(
            "shed-gates",
            allowed_for("shed-gates"),
            &dir.join("Cargo.toml"),
            &mut out,
        );
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(
            out,
            vec![
                "shed-gates: its internal dependencies are dev-dependencies only, found [dependencies] -> shed-config".to_owned()
            ]
        );
        Ok(())
    }

    fn allowed_for(name: &str) -> &'static [&'static str] {
        match super::LAYERS.iter().find(|(known, _)| *known == name) {
            Some((_, allowed)) => allowed,
            None => &[],
        }
    }

    fn temp_cargo_toml(tag: &str, text: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("shed-gates-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("Cargo.toml"), text)?;
        Ok(dir)
    }
}
