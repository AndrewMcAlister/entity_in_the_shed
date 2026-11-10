//! Python's `str()` over a JSON value, and the `str(value or "")` idiom the
//! country files and the voice catalog are read through.
//!
//! The Python source spells `entry.get("born") or ""` and `str(voice.get(key)
//! or "")` everywhere it reads authored data (`agent.py:2538`, `agent.py:3103`
//! and their neighbours). Both halves of that idiom have exact behaviour this
//! port has to keep:
//!
//! - `or ""` fires on every *falsy* value, not just `None`: `False`, `0`,
//!   `0.0`, `""`, `[]` and `{}` all collapse to the empty string too.
//! - `str()` then renders `None` as `"None"`, booleans as `"True"`/`"False"`,
//!   and numbers the way Python's `repr` does (`72.0`, not `72`).
//!
//! Plan §3.2 has no `utils/` file for this; it is the shared semantics behind
//! `utils/characters.py` and the voice half of `agent.py`, and it lives here
//! because both `shed-characters` and `shed-tts` may depend on this crate
//! while they may not depend on each other (the layer map).

use serde_json::Value;

/// Python's `str(value)` over a JSON value.
///
/// `None` renders `"None"` and booleans render `"True"`/`"False"` — the
/// spellings `str()` gives them, which a falsy-blind `format!("{value}")`
/// would get wrong. Numbers keep their authored spelling (`72` stays `72`,
/// `72.0` stays `72.0`), exactly as Python's `json.loads` + `str` round-trip
/// does for the ints and floats a country file can hold.
///
/// Arrays and objects fall back to their JSON spelling. Python's `str()`
/// would quote their strings with `'`; no shipped data reaches that branch,
/// because every field read this way is a string or a number.
#[must_use]
pub fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// Python's `repr()` over a JSON value, as the diagnostic f-strings spell it
/// (`{raw_pitch!r}`, `{voice_name!r}` in `validate_character_pitches` and
/// `validate_character_voices`).
///
/// Strings get Python's quote choice: single quotes normally, double quotes
/// only when the text holds a `'` and no `"`. The escapes kept are the ones
/// authored data can hold — backslash, both quotes, `\n`, `\r`, `\t`; other
/// control characters pass through, which no country file authors.
///
/// Numbers render through [`py_float_repr`] for floats and plain for ints,
/// exactly as Python's `repr` does; arrays and objects render Python-style
/// (`[1, 'a']`, `{'a': 1}`). Object keys come out in sorted order here — the
/// JSON value carries no insertion order — which only ever reorders the
/// members inside one repr of a nested object, never the message shape.
#[must_use]
pub fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(flag) => if *flag { "True" } else { "False" }.to_owned(),
        Value::String(text) => py_str_repr(text),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(py_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, item)| format!("{}: {}", py_str_repr(key), py_repr(item)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Python's `repr()` for a `str`: the quote and escape rules of `py_repr`'s
/// string branch.
fn py_str_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut body = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => body.push_str("\\\\"),
            '\n' => body.push_str("\\n"),
            '\r' => body.push_str("\\r"),
            '\t' => body.push_str("\\t"),
            c if c == quote => {
                body.push('\\');
                body.push(c);
            }
            c => body.push(c),
        }
    }
    format!("{quote}{body}{quote}")
}

/// Python's `repr()`/`str()` for a `float`: the shortest spelling that reads
/// back as the same value, but never without a decimal point — `200.0`, not
/// `200`.
///
/// Rust's `Display` for `f64` is already the shortest round-trip form; the
/// only gap is that Python switches to exponent notation past `1e16` and
/// below `1e-4`, while this keeps decimal. No shipped message quotes a pitch
/// that far out of range, so the gap stays inside the known-divergence note
/// in `docs/PARITY.md`.
#[must_use]
pub fn py_float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let text = value.to_string();
    if text.contains(['.', 'e', 'n']) {
        // "n" catches the inf/nan spellings Display would otherwise print.
        return text;
    }
    format!("{text}.0")
}

/// `str(value or "")`: the falsy values Python's `or` treats as absent
/// collapse to `""`, anything else stringifies through [`py_str`].
#[must_use]
pub fn text_or_empty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) if number.as_f64() == Some(0.0) => String::new(),
        Some(Value::Array(items)) if items.is_empty() => String::new(),
        Some(Value::Object(map)) if map.is_empty() => String::new(),
        Some(other) => py_str(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn str_renders_the_values_python_renders_differently() {
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(true)), "True");
        assert_eq!(py_str(&json!(false)), "False");
        assert_eq!(py_str(&json!("he")), "he");
    }

    #[test]
    fn numbers_keep_their_authored_spelling() {
        assert_eq!(py_str(&json!(1948)), "1948");
        assert_eq!(py_str(&json!(72.0)), "72.0");
        assert_eq!(py_str(&json!(72.5)), "72.5");
    }

    #[test]
    fn every_falsy_value_collapses_to_empty() {
        assert_eq!(text_or_empty(None), "");
        for falsy in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert_eq!(text_or_empty(Some(&falsy)), "", "{falsy} should be falsy");
        }
    }

    #[test]
    fn truthy_values_stringify() {
        assert_eq!(text_or_empty(Some(&json!("  Ana  "))), "  Ana  ");
        assert_eq!(text_or_empty(Some(&json!(1948))), "1948");
        assert_eq!(text_or_empty(Some(&json!(true))), "True");
        // Python's `0.0 or ""` is empty, but `-0.0` is falsy too.
        assert_eq!(text_or_empty(Some(&json!(-0.0))), "");
    }

    #[test]
    fn repr_quotes_like_python_chooses_quotes() {
        assert_eq!(py_repr(&json!("en-GB-Ryan")), "'en-GB-Ryan'");
        assert_eq!(py_repr(&json!("it's")), "\"it's\""); // ' inside, no " -> "..."
        assert_eq!(py_repr(&json!("say 'hi' \"x\"")), "'say \\'hi\\' \"x\"'");
        assert_eq!(py_repr(&json!("a\\b\nc\td")), r"'a\\b\nc\td'");
    }

    #[test]
    fn repr_renders_the_other_json_types_python_style() {
        assert_eq!(py_repr(&json!(null)), "None");
        assert_eq!(py_repr(&json!(true)), "True");
        assert_eq!(py_repr(&json!(72)), "72");
        assert_eq!(py_repr(&json!([1, "a"])), "[1, 'a']");
        assert_eq!(py_repr(&json!({"b": 1, "a": "x"})), "{'a': 'x', 'b': 1}");
    }

    #[test]
    fn float_repr_never_drops_the_decimal_point() {
        assert_eq!(py_float_repr(200.0), "200.0");
        assert_eq!(py_float_repr(72.5), "72.5");
        assert_eq!(py_float_repr(-0.0), "-0.0");
        assert_eq!(py_float_repr(f64::NAN), "nan");
        assert_eq!(py_float_repr(f64::INFINITY), "inf");
        assert_eq!(py_float_repr(f64::NEG_INFINITY), "-inf");
    }
}
