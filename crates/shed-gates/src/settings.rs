//! Settings-template gate (How_To_Code principle 19, plan §5): every name
//! `shed-config` reads must appear in `.env.example`, and every name the
//! template advertises must actually be read. Bidirectional from day 1, so
//! the template cannot rot in either direction — a key nobody reads teaches
//! operators to distrust the file, and a read key nobody documented is a
//! knob that exists only in source.
//!
//! The read set is extracted from `crates/shed-config/src/*.rs` string
//! literals that look like environment names: all-uppercase with at least
//! one underscore (`STORAGE_ROOT`, `MCTRADER_MCP_CACHE_TTL`). Two known
//! limits, both deliberate: a future underscore-free name would slip the
//! extraction — and the documented side would then fail the gate loudly,
//! which is the safe direction to fail; and `#[cfg(test)]` sections are cut
//! because test fixtures invent plausible names (`SHED_TEST_T_MISSING`)
//! that are not real settings. Comment lines are skipped for the same
//! reason — this crate's own docs quote `LMSTUDIO_MODELS_DIR` as an
//! example, and it is not a setting this app reads.
//!
//! Names the OS provides are allow-listed: code may read them from the
//! process environment, but they do not belong in `.env.example`, which
//! documents this app's own knobs. A `.env` entry for `USERNAME` would not
//! be a setting; it would be an operator shouting into the void.
//!
//! The Stage 9 runtime warning (unknown keys found in a live `.env`) is a
//! different check on different data and arrives there.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// OS-provided names exempt from the documented side of the gate.
const OS_PROVIDED: &[&str] = &["HOME", "LOCALAPPDATA", "USER", "USERNAME", "USERPROFILE"];

/// Every violation, empty when template and code agree.
pub fn violations() -> Vec<String> {
    let root = crate::workspace_root();
    let config_src = root.join("crates/shed-config/src");
    let template = root.join(".env.example");
    let text = match fs::read_to_string(&template) {
        Ok(text) => text,
        Err(error) => return vec![format!(".env.example unreadable: {error}")],
    };
    let read = read_names(&config_src);
    let documented = documented_names(&text);
    let mut out = Vec::new();
    for name in read.difference(&documented) {
        if !OS_PROVIDED.contains(&name.as_str()) {
            out.push(format!(
                "{name}: read by shed-config but missing from .env.example"
            ));
        }
    }
    for name in documented.difference(&read) {
        out.push(format!(
            "{name}: in .env.example but never read by shed-config"
        ));
    }
    out
}

/// Environment-name pattern: leading letter, then uppercases, digits and
/// underscores, with at least one underscore to separate it from message
/// fragments like `OK` or `UTF8`.
fn looks_like_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts = matches!(chars.next(), Some(c) if c.is_ascii_uppercase());
    starts
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && name.contains('_')
}

/// The read set: matching string literals across the config crate's sources.
fn read_names(config_src: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for file in crate::walk_files(config_src) {
        if file.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        out.extend(names_in_source(&text));
    }
    out
}

/// The names one source file reads. Test sections are cut; comment-only
/// lines are skipped.
fn names_in_source(text: &str) -> BTreeSet<String> {
    let code = match text.find("#[cfg(test)]") {
        Some(index) => &text[..index],
        None => text,
    };
    let mut out = BTreeSet::new();
    for line in code.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for literal in string_literals(line) {
            if looks_like_env_name(&literal) {
                out.insert(literal);
            }
        }
    }
    out
}

/// The contents of each `"..."` on one line. Good enough for this
/// workspace's style: names never contain quotes or newlines, and a
/// mis-slice produces a non-matching fragment, never a phantom key.
fn string_literals(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '"' {
            index += 1;
            continue;
        }
        index += 1;
        let mut literal = String::new();
        while index < chars.len() {
            let c = chars[index];
            index += 1;
            match c {
                '"' => break,
                '\\' => {
                    if index < chars.len() {
                        literal.push(chars[index]);
                        index += 1;
                    }
                }
                other => literal.push(other),
            }
        }
        out.push(literal);
    }
    out
}

/// The documented set: keys of the template's assignment lines.
fn documented_names(template: &str) -> BTreeSet<String> {
    template
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, _)| key.trim().to_owned())
        .filter(|key| looks_like_env_name(key))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn template_and_read_set_agree() {
        let found = super::violations();
        assert!(
            found.is_empty(),
            "settings gate violated:\n{}",
            found.join("\n")
        );
    }

    /// The extraction itself: real reads are found; test fixtures, comments
    /// and message fragments are not.
    #[test]
    fn extraction_ignores_tests_comments_and_fragments() {
        let source = concat!(
            "fn load(src: &EnvSource) -> String {\n",
            "    getenv_str(src, \"REAL_KEY\", \"\")\n",
            "}\n",
            "// COMMENTED_KEY=docs mention\n",
            "fn message() -> String { \"BAD_KEY is broken\".to_owned() }\n",
            "#[cfg(test)]\nmod tests {\n    fn t() { getenv_str(src, \"SHED_TEST_KEY\", \"\"); }\n}\n",
        );
        let names = super::names_in_source(source);
        assert!(
            names.contains("REAL_KEY"),
            "the real read must be found: {names:?}"
        );
        assert!(
            !names.contains("SHED_TEST_KEY"),
            "test fixtures are not settings: {names:?}"
        );
        assert!(
            !names.contains("COMMENTED_KEY"),
            "comments are prose: {names:?}"
        );
        assert!(
            !names.contains("BAD_KEY"),
            "message fragments name, not read: {names:?}"
        );
    }

    /// The one extraction limit: a quoted name inside a trailing comment
    /// counts as read. Pinning the behaviour so a future rewrite notices,
    /// rather than a silent drift.
    #[test]
    fn quoted_names_in_trailing_comments_count_as_read() {
        let names = super::names_in_source(
            "let x = getenv_str(src, \"A_KEY\", \"\"); // see \"MY_TRAILING_KEY\"\n",
        );
        assert!(
            names.contains("MY_TRAILING_KEY"),
            "documented limit: {names:?}"
        );
    }

    #[test]
    fn template_parsing_skips_comments_and_blanks() {
        let names = super::documented_names(
            "# COMMENTED_KEY=doc\nKEY_ONE=1\n\n  KEY_TWO = 2 \nlower_key=3\n",
        );
        assert!(names.contains("KEY_ONE"));
        assert!(names.contains("KEY_TWO"), "keys are trimmed: {names:?}");
        assert!(
            !names.contains("COMMENTED_KEY"),
            "comments are prose: {names:?}"
        );
        assert!(
            !names.contains("lower_key"),
            "names are uppercase: {names:?}"
        );
    }

    #[test]
    fn os_provided_names_are_exempt() {
        // USERPROFILE is read by nothing today, but when Stage 8 reads it
        // the gate must not demand it in the template.
        assert!(super::OS_PROVIDED.contains(&"USERPROFILE"));
        assert!(!super::OS_PROVIDED.contains(&"STORAGE_ROOT"));
    }
}
