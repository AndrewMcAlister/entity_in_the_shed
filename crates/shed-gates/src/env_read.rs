//! Env-read rule (How_To_Code principle 19, plan §5): only `shed-config`
//! reads the process environment; every other crate receives the typed
//! `AppConfig`. This gate greps every Rust source under the workspace for
//! the `std::env` read APIs, outside that one crate.
//!
//! Command-line arguments are deliberately not covered: they are not
//! environment variables, and the binary parses `--smoke` from them.

use std::path::Path;

/// The needle is assembled at runtime on purpose: were this file to contain
/// the literal, the gate would flag its own source.
pub(crate) fn needle() -> String {
    format!("env{}var", "::")
}

/// Every violation, empty when no crate outside `shed-config` reads env vars.
pub fn violations() -> Vec<String> {
    let root = crate::workspace_root();
    let needle = needle();
    let mut out = Vec::new();
    for parent in ["crates", "bins"] {
        for crate_dir in crate::child_dirs(&root.join(parent)) {
            let is_config =
                crate_dir.file_name().and_then(|name| name.to_str()) == Some("shed-config");
            if !is_config {
                scan_crate(&crate_dir, &needle, &root, &mut out);
            }
        }
    }
    out
}

fn scan_crate(crate_dir: &Path, needle: &str, root: &Path, out: &mut Vec<String>) {
    for file in crate::walk_files(&crate_dir.join("src")) {
        if file.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = crate::relative_to_root(root, &file);
        for (index, line) in text.lines().enumerate() {
            if line.contains(needle) {
                out.push(format!(
                    "{rel}:{}: environment read outside shed-config: {}",
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_crate_outside_config_reads_the_environment() {
        let found = super::violations();
        assert!(
            found.is_empty(),
            "env-read gate violated:\n{}",
            found.join("\n")
        );
    }

    /// The scanner itself fires on a planted read; the planted source is
    /// assembled at runtime so this file stays clean.
    #[test]
    fn scanner_fires_on_a_planted_read() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("shed-gates-envread-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src"))?;
        let planted = format!("fn f() {{ let _ = std::env{}var(\"X\"); }}\n", "::");
        std::fs::write(dir.join("src").join("lib.rs"), planted)?;
        let mut out = Vec::new();
        super::scan_crate(&dir, &super::needle(), &dir, &mut out);
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(out.len(), 1, "the planted read must be found: {out:?}");
        assert!(
            out[0].contains("lib.rs:1"),
            "violation names file:line: {out:?}"
        );
        Ok(())
    }
}
