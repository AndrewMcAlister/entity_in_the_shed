//! The env-reading helpers, ported from the Python app's `utils/env.py`.
//!
//! Every helper takes an [`EnvSource`], so tests inject values instead of
//! mutating process state (the Python tests used `mock.patch.dict`; the Rust
//! port needs no global mutation and stays parallel-safe).
//!
//! Not ported: `getenv_first` (no caller in the source app) and
//! `current_process_account` (arrives with the Stage 8 keyring surface).

use std::collections::BTreeMap;

use crate::envsrc::EnvSource;
use crate::ConfigError;

/// Raw string value, or `default`. No trimming: the Python callers trimmed
/// themselves, and the trim belongs to the reader that knows the shape.
#[must_use]
pub fn getenv_str(src: &EnvSource, name: &str, default: &str) -> String {
    match src.get(name) {
        Some(value) => value,
        None => default.to_owned(),
    }
}

/// Python's `getenv_int`: a present value clamps to `>= 0` when it parses and
/// falls back unclamped when it does not; a missing value falls back clamped,
/// because Python runs the same `max(0, ...)` over the default there.
#[must_use]
pub fn getenv_int(src: &EnvSource, name: &str, default: i64) -> i64 {
    match src.get(name) {
        None => default.max(0),
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(value) => value.max(0),
            Err(_) => default,
        },
    }
}

/// Python's `getenv_float`. Not clamped: range knobs are validated by the
/// caller that knows the server's per-parameter bounds.
#[must_use]
pub fn getenv_float(src: &EnvSource, name: &str, default: f64) -> f64 {
    match src.get(name) {
        None => default,
        Some(raw) => match raw.trim().parse::<f64>() {
            Ok(value) => value,
            Err(_) => default,
        },
    }
}

/// Python's `getenv_timeout_seconds`: the disable words mean "no timeout"
/// (`None`), a real value floors at 1 second, garbage falls back to the
/// default (which may itself be `None`).
#[must_use]
pub fn getenv_timeout_seconds(src: &EnvSource, name: &str, default: Option<i64>) -> Option<i64> {
    let raw = match src.get(name) {
        Some(raw) => raw,
        None => return default,
    };
    let normalized = raw.trim().to_lowercase();
    if [
        "", "0", "none", "false", "off", "disable", "disabled", "infinite", "unlimited",
    ]
    .contains(&normalized.as_str())
    {
        return None;
    }
    match normalized.parse::<i64>() {
        Ok(value) => Some(value.max(1)),
        Err(_) => default,
    }
}

/// Python's `parse_bool` for string values: anything not in the false set is
/// true, including garbage — the source app treats a typo'd flag as "on"
/// rather than silently disabling the feature.
#[must_use]
pub fn parse_bool(value: Option<String>, default: bool) -> bool {
    let raw = match value {
        Some(raw) => raw,
        None => return default,
    };
    let normalized = raw.trim().to_lowercase();
    !["", "0", "false", "no", "off"].contains(&normalized.as_str())
}

/// Python's `normalize_AI_MODEL_name`: a registry name ending `:latest`
/// drops the tag (`registry:5000/model:latest` → `registry:5000/model`);
/// plain `model:latest` keeps it.
#[must_use]
pub fn normalize_model_name(value: &str, default: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return default.to_owned();
    }
    let colons = trimmed.matches(':').count();
    if trimmed.ends_with(":latest") && colons > 1 {
        // Safe slicing: the suffix match guarantees a ':' boundary in between.
        return trimmed[..trimmed.len() - ":latest".len()].to_owned();
    }
    trimmed.to_owned()
}

/// Python's `parse_json_env` for a list. Blank is the empty list; invalid
/// JSON or a non-list is a startup failure naming the key (principle 14 —
/// Python deferred this to the first MCP call, where it surfaced as a `/health` 500).
pub fn parse_json_list(name: &str, raw: &str) -> Result<Vec<String>, ConfigError> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value = parse_value(name, raw)?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        _ => return Err(decode_error(name, "a JSON list")),
    };
    items
        .into_iter()
        .map(|item| match item {
            serde_json::Value::String(text) => Ok(text),
            _ => Err(decode_error(name, "a JSON list of strings")),
        })
        .collect()
}

/// Python's `parse_json_env` for a dict of strings.
pub fn parse_json_map(name: &str, raw: &str) -> Result<BTreeMap<String, String>, ConfigError> {
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let value = parse_value(name, raw)?;
    let object = match value {
        serde_json::Value::Object(object) => object,
        _ => return Err(decode_error(name, "a JSON object")),
    };
    object
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(text) => Ok((key, text)),
            _ => Err(decode_error(name, "a JSON object of strings")),
        })
        .collect()
}

fn parse_value(name: &str, raw: &str) -> Result<serde_json::Value, ConfigError> {
    serde_json::from_str(raw).map_err(|error| {
        ConfigError(format!("{name} must be valid JSON: {error}"))
    })
}

fn decode_error(name: &str, expected: &str) -> ConfigError {
    ConfigError(format!("{name} must decode to {expected}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_file_values(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn parse_bool_matches_python() {
        assert!(!parse_bool(None, false));
        assert!(parse_bool(None, true));
        for truthy in ["1", "true", "yes", "on", "TRUE", "anything"] {
            assert!(parse_bool(Some(truthy.to_owned()), false), "{truthy}");
        }
        for falsy in ["0", "false", "no", "off", "", "   "] {
            assert!(!parse_bool(Some(falsy.to_owned()), true), "{falsy}");
        }
    }

    #[test]
    fn getenv_int_matches_python() {
        assert_eq!(getenv_int(&src(&[("K", "42")]), "K", 7), 42);
        assert_eq!(getenv_int(&src(&[("K", "-5")]), "K", 7), 0);
        assert_eq!(getenv_int(&src(&[("K", "abc")]), "K", 7), 7);
        assert_eq!(getenv_int(&src(&[]), "SHED_TEST_INT_MISSING", 9), 9);
        // Python clamps the missing-value default through max(0, default).
        assert_eq!(getenv_int(&src(&[]), "SHED_TEST_INT_MISSING", -3), 0);
    }

    #[test]
    fn getenv_float_matches_python() {
        assert_eq!(getenv_float(&src(&[("K", "1.05")]), "K", 0.5), 1.05);
        assert_eq!(getenv_float(&src(&[("K", "2")]), "K", 0.5), 2.0);
        assert_eq!(getenv_float(&src(&[("K", "-0.25")]), "K", 0.5), -0.25);
        assert_eq!(getenv_float(&src(&[("K", "abc")]), "K", 0.5), 0.5);
        assert_eq!(getenv_float(&src(&[]), "SHED_TEST_FLOAT_MISSING", 1.75), 1.75);
    }

    #[test]
    fn getenv_timeout_matches_python() {
        let none_default: Option<i64> = None;
        assert_eq!(getenv_timeout_seconds(&src(&[]), "SHED_TEST_T_MISSING", Some(30)), Some(30));
        for disabled in ["0", "none", "off", "disabled", "infinite", "unlimited", ""] {
            assert_eq!(
                getenv_timeout_seconds(&src(&[("K", disabled)]), "K", Some(30)),
                none_default,
                "{disabled}"
            );
        }
        assert_eq!(getenv_timeout_seconds(&src(&[("K", "25")]), "K", Some(30)), Some(25));
        assert_eq!(getenv_timeout_seconds(&src(&[("K", "1")]), "K", Some(30)), Some(1));
        assert_eq!(getenv_timeout_seconds(&src(&[("K", "-4")]), "K", Some(30)), Some(1));
        assert_eq!(getenv_timeout_seconds(&src(&[("K", "abc")]), "K", Some(30)), Some(30));
    }

    #[test]
    fn model_name_normalisation_matches_python() {
        assert_eq!(
            normalize_model_name("registry:5000/model:latest", "d"),
            "registry:5000/model"
        );
        assert_eq!(
            normalize_model_name("myrepo/my-model:latest", "d"),
            "myrepo/my-model:latest"
        );
        assert_eq!(normalize_model_name("model:latest", "d"), "model:latest");
        assert_eq!(normalize_model_name("  ", "d"), "d");
        assert_eq!(normalize_model_name("", "x:y"), "x:y");
        assert_eq!(normalize_model_name("  abc  ", "d"), "abc");
    }

    #[test]
    fn json_env_helpers_match_python() -> Result<(), ConfigError> {
        assert!(parse_json_list("X", "")?.is_empty());
        assert!(parse_json_map("X", "   ")?.is_empty());
        assert_eq!(parse_json_list("X", r#"["a", "b"]"#)?, vec!["a", "b"]);
        let map = parse_json_map("X", r#"{"k": "1"}"#)?;
        assert_eq!(map.get("k").map(String::as_str), Some("1"));

        let err = match parse_json_map("MY_VAR", r#"["a"]"#) {
            Ok(_) => return Err(ConfigError("expected a failure".to_owned())),
            Err(error) => error,
        };
        assert!(err.0.contains("MY_VAR must decode to"), "{err}");

        let err = match parse_json_list("MY_VAR", "not json") {
            Ok(_) => return Err(ConfigError("expected a failure".to_owned())),
            Err(error) => error,
        };
        assert!(err.0.contains("MY_VAR must be valid JSON"), "{err}");
        Ok(())
    }
}
