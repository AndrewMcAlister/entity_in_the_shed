//! `How_To_Code` principle 24, applied to `.env.example`: every setting the code reads is
//! documented, nothing documented is left unread, every documented value is the default the code
//! actually produces, and every name is spelled the way the principle spells it.
//!
//! The Python app keeps 150 environment names in module-level globals and its `.env.example`
//! documents 113 of them, so 36 settings can be mis-set with nothing in the repository saying what
//! they do. The clone fixes that by construction: `shed-config` is the only crate that reads the
//! environment (`layers` checks that) and `.env.example` is the one file that describes it. These
//! tests keep the fix true as stages add settings, so a stage that reads a new name has to
//! document it in the same commit that reads it.
//!
//! ## How the read names are found
//!
//! The scan reads `shed-config`'s shipped files and records each `env.<method>("NAME")` call,
//! which is the shape every read in that crate takes:
//!
//! - the receiver is `env`, so a test helper's `table.get("AI_MODEL")` and a JSON map lookup like
//!   `server_env.get("TOKEN")` are not setting reads;
//! - the name is a string literal. A read that passes a computed name is reported as a problem,
//!   because a name built at run time is a setting no template can be checked against;
//! - a comment line is not code, and everything from a module's `#[cfg(test)]` line down is test
//!   code, which is not shipped. Every module in this workspace keeps its tests as the last block.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use crate::layers::collect_rs;

/// The `Env` methods that read a setting. A new one means the settings it reads are new too.
pub const ENV_READ_METHODS: [&str; 3] = [".get(", ".text(", ".int("];

/// The receiver every shipped read uses. Requiring it is what keeps a map lookup out of the list.
pub const ENV_RECEIVER: &str = "env";

/// Names the app reads from the machine rather than from the operator. `LOCALAPPDATA` and `HOME`
/// locate the default storage root and are set by Windows and the shell, so writing them in
/// `.env.example` would advertise settings nobody is meant to set.
pub const MACHINE_NAMES: [&str; 2] = ["LOCALAPPDATA", "HOME"];

/// What one crate's shipped code reads.
#[derive(Debug, Default)]
pub struct Reads {
    /// The settings it names.
    pub names: BTreeSet<String>,
    /// `file:line` for each read whose name is not a literal, which the gate reports rather than
    /// ignores.
    pub unnamed: Vec<String>,
}

/// The settings `shed-config` reads, found by scanning its shipped source.
pub fn crate_reads(package_dir: &Path) -> io::Result<Reads> {
    let mut files = Vec::new();
    collect_rs(&package_dir.join("src"), &mut files)?;
    files.sort();

    let mut reads = Reads::default();
    for path in files {
        let code = fs::read_to_string(&path)?;
        for (number, raw) in code.lines().enumerate() {
            let line = raw.trim();
            if line.starts_with("//") {
                continue;
            }
            if line == "#[cfg(test)]" {
                break;
            }

            for method in ENV_READ_METHODS.iter().copied() {
                for at in line.match_indices(method).map(|(at, _)| at) {
                    match read_setting_name(&line[..at], &line[at + method.len()..]) {
                        Some(Some(name)) => {
                            reads.names.insert(name);
                        }
                        Some(None) => {
                            reads.unnamed.push(format!("{}:{}", path.display(), number + 1))
                        }
                        None => {}
                    }
                }
            }
        }
    }
    Ok(reads)
}

/// What one `env.<method>(` reads, given the text on each side of it: `Some(Some(NAME))` when it
/// names a setting, `Some(None)` when the name is computed, and `None` when this method call is
/// not a setting read at all.
pub fn read_setting_name(before: &str, after: &str) -> Option<Option<String>> {
    names_the_receiver(before).then(|| literal_name(after).map(str::to_string))
}

/// Whether the code before a `.method(` is the receiver, `env`, and not the tail of a longer
/// identifier like `base_env`.
fn names_the_receiver(before: &str) -> bool {
    let Some(at) = before.rfind(ENV_RECEIVER) else {
        return false;
    };
    // `env` has to stand on its own: the tail of `base_env` is a different receiver. Whatever
    // follows it is whitespace or nothing, because the `.method(` was cut off at `at`.
    let tail_of_a_longer_name = at != 0 && is_identifier_char(before.as_bytes()[at - 1]);
    !tail_of_a_longer_name && before[at + ENV_RECEIVER.len()..].chars().all(char::is_whitespace)
}

fn is_identifier_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The setting name a read names, or `None` when the argument is not a quoted name.
fn literal_name(after: &str) -> Option<&str> {
    let text = after.strip_prefix('"')?;
    let at = text.find('"')?;
    is_setting_name(&text[..at]).then_some(&text[..at])
}

/// Principle 24's spelling of a setting name: uppercase, starting with a letter.
pub fn is_setting_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    first.is_ascii_uppercase()
        && chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

/// The names `.env.example` sets, read with the clone's own `.env` reader.
///
/// `Env::default()` is empty, so the table holds exactly what the file says and nothing the
/// launcher happened to set.
pub fn documented_names(root: &Path) -> BTreeSet<String> {
    let env = shed_config::Env::default().with_dotenv_overrides(&root.join(".env.example"));
    env.names().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{package_manifests, package_name};
    use crate::repo_root;
    use shed_config::{AppConfig, LogLevel};
    use std::path::PathBuf;

    /// The directory of the named package, found from the workspace manifests rather than from a
    /// path written down here.
    fn package_dir(root: &Path, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        for manifest in package_manifests(root)? {
            if package_name(&manifest)? == name {
                return manifest.parent().map(PathBuf::from).ok_or_else(|| {
                    Box::from(format!("{} has no parent directory", manifest.display()))
                });
            }
        }
        Err(Box::from(format!("{name} is not a package in this workspace")))
    }

    fn config_reads() -> Result<Reads, Box<dyn std::error::Error>> {
        Ok(crate_reads(&package_dir(&repo_root(), "shed-config")?)?)
    }

    /// A directory of its own under the approved temporary tree, so the gate writes nothing in the
    /// repository (principle 20) and the two configs it compares cannot collide.
    fn scratch(label: &str) -> io::Result<PathBuf> {
        let dir = std::env::temp_dir().join("shed-gates-settings-tests").join(label);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn every_setting_the_app_reads_is_documented() -> crate::Gate {
        let reads = config_reads()?;
        let documented = documented_names(&repo_root());

        let missing: Vec<&str> = reads
            .names
            .iter()
            .filter(|name| !documented.contains(*name) && !MACHINE_NAMES.contains(&name.as_str()))
            .map(String::as_str)
            .collect();

        if !missing.is_empty() || !reads.unnamed.is_empty() {
            return Err(Box::from(format!(
                ".env.example does not cover the settings the code reads (principle 24):\n{}\n\
                 reads that do not name their setting: {}",
                missing.join("\n"),
                reads.unnamed.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn nothing_in_the_template_is_left_unread() -> crate::Gate {
        let reads = config_reads()?;
        let documented = documented_names(&repo_root());

        let stale: Vec<&str> = documented
            .iter()
            .filter(|name| !reads.names.contains(*name))
            .map(String::as_str)
            .collect();

        if !stale.is_empty() {
            return Err(Box::from(format!(
                ".env.example documents settings no code reads, which is a key someone will set \
                 and never use:\n{}",
                stale.join("\n")
            )));
        }
        Ok(())
    }

    #[test]
    fn every_documented_value_is_the_default_the_code_produces() -> crate::Gate {
        let base = scratch("defaults")?;

        // The two configurations differ only in where their values come from: one has nothing set,
        // the other has exactly what `.env.example` says. If the template is honest, the app built
        // from it is the app built from nothing.
        let unset = AppConfig::load_from(&shed_config::Env::default(), &base)?;
        let documented = AppConfig::load_from(
            &shed_config::Env::default().with_dotenv_overrides(&repo_root().join(".env.example")),
            &base,
        )?;

        let problems = compare(&unset, &documented, &base);
        if !problems.is_empty() {
            return Err(Box::from(format!(
                ".env.example sets a value that is not the default the code uses:\n{}",
                problems.join("\n")
            )));
        }
        Ok(())
    }

    /// Field by field, because `AppConfig` deliberately does not derive `PartialEq`: the storage
    /// root is a path that exists on disk, and comparing whole structs would compare the scratch
    /// directory the two configs were built in.
    fn compare(unset: &AppConfig, documented: &AppConfig, base: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        let mut check = |field: &str, left: String, right: String| {
            if left != right {
                problems.push(format!(
                    "{field}: the code defaults to {left:?}, the file says {right:?}"
                ));
            }
        };

        // A configured `STORAGE_ROOT` is resolved against the base directory, so the part that
        // says which directory was chosen is the part after it.
        let relative = |root: &Path| match root.strip_prefix(base) {
            Ok(rest) => rest.display().to_string(),
            Err(_) => root.display().to_string(),
        };
        check("STORAGE_ROOT", relative(&unset.storage.root), relative(&documented.storage.root));

        check("APP_HOST", unset.server.host.clone(), documented.server.host.clone());
        check("APP_PORT", unset.server.port.to_string(), documented.server.port.to_string());
        check("AI_MODEL", unset.models.ai_model.clone(), documented.models.ai_model.clone());
        check(
            "LM_ENGINE_NORMAL_MODEL",
            unset.models.normal_model.clone(),
            documented.models.normal_model.clone(),
        );
        check(
            "MCTRADER_MCP_SERVER_NAME",
            unset.mcp.server_name.clone(),
            documented.mcp.server_name.clone(),
        );
        check(
            "MCTRADER_MCP_TRANSPORT",
            unset.mcp.transport_text(),
            documented.mcp.transport_text(),
        );
        check(
            "MCTRADER_MCP_TIMEOUT",
            unset.mcp.timeout_seconds.to_string(),
            documented.mcp.timeout_seconds.to_string(),
        );
        check("MCTRADER_MCP_COMMAND", unset.mcp.command.clone(), documented.mcp.command.clone());
        check(
            "MCTRADER_MCP_ARGS",
            format!("{:?}", unset.mcp.args),
            format!("{:?}", documented.mcp.args),
        );
        check(
            "MCTRADER_MCP_ENV",
            format!("{:?}", unset.mcp.server_env),
            format!("{:?}", documented.mcp.server_env),
        );
        check("MCTRADER_MCP_URL", unset.mcp.url.clone(), documented.mcp.url.clone());
        check("LOG_LEVEL", format!("{:?}", unset.log_level), format!("{:?}", documented.log_level));

        problems
    }

    #[test]
    fn every_documented_name_is_spelled_as_a_setting() -> crate::Gate {
        let template = repo_root().join(".env.example");
        let mut problems = Vec::new();

        for (number, raw) in fs::read_to_string(&template)?.lines().enumerate() {
            let line = raw.trim();
            // A blank line separates the groups of settings, which is the same thing the `.env`
            // grammar does with it.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                problems.push(format!("line {} is neither a comment nor a setting", number + 1));
                continue;
            };
            if !is_setting_name(name) {
                problems.push(format!(
                    "{name} on line {} is not an UPPERCASE setting name",
                    number + 1
                ));
            }
            // Principle 24's other half, checked before the first percentage key arrives so that a
            // later stage cannot add one in the wrong shape: quoted, and the value ends in %.
            if name.ends_with("_PERCENT")
                && !(value.trim_start().starts_with('"') && value.trim_end().ends_with("%\""))
            {
                problems.push(format!(
                    "{name} on line {} must hold a double-quoted value ending in %",
                    number + 1
                ));
            }
        }

        if !problems.is_empty() {
            return Err(Box::from(format!("{}:\n{}", template.display(), problems.join("\n"))));
        }
        Ok(())
    }

    /// The scan is only a gate if it actually finds reads. This feeds it the shapes it has to tell
    /// apart: a read that names a setting, a map lookup that looks like one, a receiver whose name
    /// only ends in `env`, a read that only appears in a comment, and a read whose name is
    /// computed at run time.
    #[test]
    fn the_scan_finds_a_named_read_and_ignores_the_lookalikes() {
        let cases: [(&str, Option<Option<&str>>); 7] = [
            ("let host = env.text(\"APP_HOST\", DEFAULT_HOST);", Some(Some("APP_HOST"))),
            (
                "let timeout = env.int(\"MCTRADER_MCP_TIMEOUT\", DEFAULT);",
                Some(Some("MCTRADER_MCP_TIMEOUT")),
            ),
            ("let level = env.get(\"LOG_LEVEL\").unwrap_or_default();", Some(Some("LOG_LEVEL"))),
            // A map lookup, a receiver whose name only ends in `env`, and a line that only holds
            // the read inside a comment: none of them is a setting.
            ("assert_eq!(stdio.server_env.get(\"TOKEN\"), Some(&Value::from(\"t\")));", None),
            ("let other = base_env.text(\"OTHER\", \"\");", None),
            ("// env.text(\"NOT_A_SETTING\", \"\")", None),
            // A read whose name is built at run time is a setting the template cannot be checked
            // against, so it is reported rather than skipped.
            ("let value = env.get(name);", Some(None)),
        ];

        for (line, expected) in cases {
            let expected = expected.map(|name| name.map(String::from));
            assert_eq!(read_name_in(line), expected, "{line}");
        }
    }

    /// What [`read_setting_name`] found on a line, in the shape [`crate_reads`] uses: `None` when
    /// the line holds no setting read, `Some(None)` when it holds one whose name is computed.
    fn read_name_in(line: &str) -> Option<Option<String>> {
        if line.trim_start().starts_with("//") {
            return None;
        }
        ENV_READ_METHODS.iter().copied().find_map(|method| {
            line.match_indices(method)
                .map(|(at, _)| at)
                .find_map(|at| read_setting_name(&line[..at], &line[at + method.len()..]))
        })
    }

    #[test]
    fn a_setting_name_is_uppercase() {
        for name in ["APP_PORT", "MCTRADER_MCP_SERVER_NAME", "LOCALAPPDATA", "LOG_LEVEL"] {
            assert!(is_setting_name(name), "{name}");
        }
        for name in ["app_port", "1APP", "", "APP_PORT2 x", "App_Port"] {
            assert!(!is_setting_name(name), "{name}");
        }
    }

    #[test]
    fn the_log_level_default_survives_an_absent_setting() {
        // A shape the value comparison above cannot see on its own: `LOG_LEVEL` is only read when
        // the name is present, so a template that dropped it would still match the default. The
        // first test catches that as an undocumented name; this one records why it matters.
        assert!(documented_names(&repo_root()).contains("LOG_LEVEL"));
        assert_eq!(LogLevel::from_setting(&shed_config::Env::default()), LogLevel::Info);
    }

    /// Principle 19, and `docs/PARITY.md` fix 11: the file holding this machine's settings is
    /// never committed, and the file describing them always is.
    #[test]
    fn the_real_env_file_is_ignored_and_the_template_is_not() -> crate::Gate {
        let ignore = repo_root().join(".gitignore");
        let patterns: Vec<String> = fs::read_to_string(&ignore)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(String::from)
            .collect();

        let ignores_env = patterns
            .iter()
            .any(|pattern| pattern == ".env" || pattern == "/.env" || pattern == ".env*");
        if !ignores_env {
            return Err(Box::from(format!(
                "{} does not ignore `.env`, so a machine's real settings could be committed",
                ignore.display()
            )));
        }

        // `.env*` is in this list on purpose: it is the pattern that ignores the template while
        // looking like it only ignores the real file.
        for pattern in [".env.example", "/.env.example", ".env*"] {
            if patterns.iter().any(|line| line == pattern) {
                return Err(Box::from(format!(
                    "{pattern} in {} ignores `.env.example`, which is the one settings file this \
                     repository is meant to carry",
                    ignore.display()
                )));
            }
        }
        Ok(())
    }
}
