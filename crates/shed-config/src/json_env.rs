//! A setting whose value is a JSON document.
//!
//! Ported from `utils/env.py:83` (`parse_json_env`).

use serde_json::{Map, Value};

use crate::ConfigError;

/// `parse_json_env` for a list-valued setting (`MCTRADER_MCP_ARGS`).
///
/// Blank means an empty list rather than an error: the setting is optional, and "no arguments"
/// is what an empty line in `.env` says.
///
/// Crate-internal: a JSON-valued setting is read here and nowhere else, so the helper never
/// leaves this crate (`How_To_Code` principle 19).
pub(crate) fn parse_json_list(name: &str, raw: Option<&str>) -> Result<Vec<Value>, ConfigError> {
    match parse_json_value(name, raw)? {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(ConfigError::wrong_kind(name, "list")),
    }
}

/// `parse_json_env` for an object-valued setting (`MCTRADER_MCP_ENV`).
pub(crate) fn parse_json_object(
    name: &str,
    raw: Option<&str>,
) -> Result<Map<String, Value>, ConfigError> {
    match parse_json_value(name, raw)? {
        None => Ok(Map::new()),
        Some(Value::Object(entries)) => Ok(entries),
        Some(_) => Err(ConfigError::wrong_kind(name, "dict")),
    }
}

/// The shared half of both helpers: blank is nothing, anything else has to parse.
fn parse_json_value(name: &str, raw: Option<&str>) -> Result<Option<Value>, ConfigError> {
    let Some(text) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };

    serde_json::from_str::<Value>(text)
        .map(Some)
        .map_err(|err| ConfigError::invalid_json(name, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_returns_an_empty_value_of_the_expected_kind() -> Result<(), ConfigError> {
        assert!(parse_json_list("X", Some(""))?.is_empty());
        assert!(parse_json_list("X", Some("   "))?.is_empty());
        assert!(parse_json_list("X", None)?.is_empty());
        assert!(parse_json_object("X", Some(""))?.is_empty());
        assert!(parse_json_object("X", Some("   "))?.is_empty());
        assert!(parse_json_object("X", None)?.is_empty());
        Ok(())
    }

    #[test]
    fn a_valid_document_decodes() -> Result<(), ConfigError> {
        let list = parse_json_list("X", Some(r#"["a", "b"]"#))?;
        assert_eq!(list, vec![Value::String("a".into()), Value::String("b".into())]);

        let object = parse_json_object("X", Some(r#"{"k": 1}"#))?;
        assert_eq!(object.get("k"), Some(&Value::from(1)));
        Ok(())
    }

    #[test]
    fn the_wrong_kind_names_the_setting_and_the_expected_kind() {
        let error = parse_json_object("MY_VAR", Some(r#"["a"]"#)).err();
        assert!(
            matches!(&error, Some(ConfigError::WrongKind { name, expected }) if name == "MY_VAR" && *expected == "dict"),
            "{error:?}"
        );
        let message = error.map(|err| err.to_string()).unwrap_or_default();
        assert!(
            message.contains("MY_VAR must decode to a dict"),
            "the message must read as the Python one does: {message}"
        );
    }

    #[test]
    fn invalid_json_names_the_setting() {
        let error = parse_json_list("MY_VAR", Some("not json")).err();
        assert!(
            matches!(&error, Some(ConfigError::InvalidJson { name, .. }) if name == "MY_VAR"),
            "{error:?}"
        );
        let message = error.map(|err| err.to_string()).unwrap_or_default();
        assert!(
            message.contains("MY_VAR must be valid JSON"),
            "the message must read as the Python one does: {message}"
        );
    }
}
