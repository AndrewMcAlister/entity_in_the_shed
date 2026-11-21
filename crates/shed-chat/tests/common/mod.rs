//! Shared helpers for the `shed-chat` test binaries.
//!
//! Each `tests/*.rs` is its own crate, so the fixture loader lives here once
//! rather than being copied into all six (the precedent is
//! `crates/shed-characters/tests/common/mod.rs`).
#![allow(dead_code)]

// The workspace denies `unused_crate_dependencies`, which judges each test
// target on its own. These three are dependencies of the crate under test and
// of this loader's own callers, not of every test binary; `unused_crate_dependencies`
// does not count `mod common;` as a use of them, so they are named here once.
use fancy_regex as _;
use shed_text as _;
use tracing as _;

use serde_json::Value;
use std::path::Path;

/// Read `tests/fixtures/<name>` — one of the D8 golden files captured from the
/// reference app's canonical source.
///
/// Every case in them is an assertion: the reference's own output, never a
/// Rust result copied sideways.
pub fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// The `cases`-style list a fixture section holds, as an array.
pub fn cases<'a>(fixture: &'a Value, section: &str) -> Vec<&'a Value> {
    fixture[section]
        .as_array()
        .unwrap_or_else(|| panic!("fixture section {section} is missing"))
        .iter()
        .collect()
}

/// A case's field, as a `&str`.
pub fn field<'a>(case: &'a Value, key: &str) -> &'a str {
    case[key]
        .as_str()
        .unwrap_or_else(|| panic!("case field {key} is not a string: {case}"))
}

/// A case's field, as a `&Value`.
pub fn value<'a>(case: &'a Value, key: &str) -> &'a Value {
    &case[key]
}

/// The `input` of a one-argument case, as the text the Python passes.
///
/// `None` is the fixture's `null`, which the source reads as `""` — the two
/// are only the same assertion once the function takes a `&str`.
pub fn input_text<'a>(case: &'a Value, key: &str) -> &'a str {
    match &case[key] {
        Value::Null => "",
        other => other
            .as_str()
            .unwrap_or_else(|| panic!("case field {key} is neither a string nor null: {other}")),
    }
}

/// The `expected` of a boolean case.
pub fn expected_bool(case: &Value, key: &str) -> bool {
    case[key]
        .as_bool()
        .unwrap_or_else(|| panic!("case field {key} is not a bool: {case}"))
}

/// The `expected` of a string case, `null` read as `""` where the source
/// would have said `str(x or "")`.
pub fn expected_text<'a>(case: &'a Value, key: &str) -> &'a str {
    match &case[key] {
        Value::Null => "",
        other => other
            .as_str()
            .unwrap_or_else(|| panic!("case field {key} is not a string: {other}")),
    }
}

/// A list-of-strings case field.
pub fn text_list<'a>(case: &'a Value, key: &str) -> Vec<&'a str> {
    case[key]
        .as_array()
        .unwrap_or_else(|| panic!("case field {key} is not an array: {case}"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("case field {key} holds a non-string: {case}"))
        })
        .collect()
}

/// A list-of-optional-strings case field: the fixture spells a `None` entry
/// `null`, and the Rust port of `Optional[str]` needs to tell them apart.
pub fn opt_text_list<'a>(case: &'a Value, key: &str) -> Vec<Option<&'a str>> {
    case[key]
        .as_array()
        .unwrap_or_else(|| panic!("case field {key} is not an array: {case}"))
        .iter()
        .map(|item| match item {
            Value::Null => None,
            other => Some(
                other
                    .as_str()
                    .unwrap_or_else(|| panic!("case field {key} holds a non-string: {case}")),
            ),
        })
        .collect()
}

/// The chunk list of a streaming case: `null` entries are the source's
/// `None` chunks, which every `feed` accepts.
pub fn chunks<'a>(case: &'a Value, key: &str) -> Vec<Option<&'a str>> {
    opt_text_list(case, key)
}
