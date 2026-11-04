//! The environment, read once, in one place.
//!
//! Ported from `utils/env.py`. `How_To_Code` principles 19 and 24: nothing outside this
//! crate touches the environment, and every setting that reaches the rest of the workspace
//! arrives as a typed value on `AppConfig` under an UPPERCASE environment name.
//!
//! The Python helpers read `os.environ` directly. Here the values live in an [`Env`] built
//! once at startup, which keeps the ported tests deterministic: they hand the helper a table
//! of values instead of mutating a process-global environment from several threads at once.
//!
//! Only the helpers a stage actually calls are carried (`How_To_Code` principle 16), which is
//! why this file is shorter than `utils/env.py`:
//!
//! | `utils/env.py` | Where it goes |
//! |---|---|
//! | `resolve_storage_dir` | here, in `storage.rs` |
//! | `getenv_int` | here, as `Env::int` |
//! | `parse_json_env` | here, in `json_env.rs` |
//! | `normalize_AI_MODEL_name` | here, in `models.rs` |
//! | `parse_bool` | Stage 2, with `LM_ENGINE_STREAM_CHAT` (`agent.py:495`), its first call site |
//! | `getenv_float` | Stage 2, with the inference-server settings that call it |
//! | `getenv_timeout_seconds` | Stage 2, with `LM_ENGINE_REPLY_TIMEOUT`, its only call site |
//! | `current_process_account` | Stage 8, with `/api/keyring` (`agent.py:8782`) |
//! | `getenv_first` | nowhere: `agent.py` imports it and no code in the source calls it |
//!
//! Two details `parse_bool` will carry when it arrives. Its off spellings are `utils/env.py:53`
//! verbatim — `""`, `"0"`, `"false"`, `"no"`, `"off"` — and they are a deny-list rather than an
//! allow-list, which is why a value like `disabled` reads as *on*: the helper asks "is anything
//! set", not "is this word a negative". And Python passes it non-strings too (`parse_bool(1)`,
//! `parse_bool(0)`) at the call sites that hand it a JSON payload value, which those stages have
//! already typed by the time they get here.

use std::collections::BTreeMap;
use std::path::Path;

/// The settings this process was started with.
#[derive(Debug, Clone, Default)]
pub struct Env {
    values: BTreeMap<String, String>,
}

impl Env {
    /// The process environment, captured once. `STORAGE_ROOT` and friends are read here and
    /// nowhere else.
    pub fn from_process() -> Self {
        Self { values: std::env::vars().collect() }
    }

    /// A table of values, for tests and for callers that already hold a parsed `.env`.
    pub fn with_values<I: IntoIterator<Item = (String, String)>>(values: I) -> Self {
        Self { values: values.into_iter().collect() }
    }

    /// `load_dotenv(BASE_DIR / ".env", override=True)` from `agent.py:299`: the file wins over
    /// a value the launcher happened to set, and a missing file is not an error.
    ///
    /// The file is read as a list of pairs rather than pushed into the process environment,
    /// which keeps the override inside this table. Entries are applied in file order, so a
    /// name that appears twice ends up with the last value, which is what `override=True`
    /// does to `os.environ`.
    pub fn with_dotenv_overrides(mut self, path: &Path) -> Self {
        for (name, value) in crate::dotenv::dotenv_entries(path) {
            self.values.insert(name, value);
        }
        self
    }

    /// Every name this table holds, in name order.
    ///
    /// The settings gate in `shed-gates` uses it to check `.env.example` against the names this
    /// crate reads. Nothing in the app enumerates its own settings, because `agent.py` never
    /// could either: its settings are 150 globals, not a table it can list.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// `os.getenv(name)`: `None` only when the name is absent. A name set to an empty string
    /// is present, and the difference decides defaults like `ENTITY_DRY_RUN`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// The value as text, or `default` when the name is absent. Mirrors
    /// `os.getenv(name, default)`; the value is returned unstripped, because the call sites
    /// differ about trimming: `MCTRADER_MCP_*` are stripped, `LM_ENGINE_HOST` rstrips a slash
    /// rather than whitespace, and `FLASK_HOST` is used raw.
    pub fn text(&self, name: &str, default: &str) -> String {
        self.get(name).map_or_else(|| default.to_string(), String::from)
    }

    /// `getenv_int`: a number, never below zero, `default` when unparseable or absent.
    ///
    /// The clamp is what the Python version does (`max(0, int(raw))`) and it is deliberate:
    /// every caller of this helper sizes a queue, a pool or a window, and a negative value
    /// from a hand-edited `.env` would turn into an empty one rather than a crash.
    pub fn int(&self, name: &str, default: i64) -> i64 {
        let raw = self.text(name, &default.to_string());
        match raw.trim().parse::<i64>() {
            Ok(value) => value.max(0),
            Err(_) => default,
        }
    }

    // `Env::text` covers `os.getenv(name, default)` and `Env::get` covers `os.getenv(name)`.
    // The remaining helpers of `utils/env.py` are listed in this module's header.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> Env {
        Env::with_values(
            pairs.iter().map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        )
    }

    #[test]
    fn getenv_int_parses_clamps_and_falls_back() {
        assert_eq!(env_with(&[("TEST_INT", "42")]).int("TEST_INT", 7), 42);
        assert_eq!(env_with(&[("TEST_INT", "-5")]).int("TEST_INT", 7), 0);
        assert_eq!(env_with(&[("TEST_INT", "abc")]).int("TEST_INT", 7), 7);
        assert_eq!(env_with(&[]).int("TEST_INT_MISSING", 9), 9);
        assert_eq!(env_with(&[("TEST_INT", " 3 ")]).int("TEST_INT", 7), 3);
    }

    #[test]
    fn an_empty_value_is_present_not_absent() {
        // `AI_MODEL=` in `.env` is how the operator says "no model of its own". The difference
        // between an empty value and an absent name is what `get` reports, and it is the whole
        // difference between a cleared setting and an unset one.
        let table = env_with(&[("AI_MODEL", "")]);
        assert_eq!(table.get("AI_MODEL"), Some(""));
        assert_eq!(table.get("MISSING_FLAG"), None);
    }
}
