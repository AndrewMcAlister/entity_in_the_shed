//! The reply document: what the model's text becomes before anything reads it.
//!
//! Five functions, one fixture: `parse_structured_candidate`,
//! `parse_model_reply_document`, `parse_model_json_content`,
//! `settle_prose_tool_calls` and `reply_without_prose_tool_calls`, every row of
//! `document.json` captured from the reference app.
//!
//! The reference app has no test of its own for any of the five — it tests them
//! only from the outside, through `test_reply_recovery.py` and
//! `test_api_chat.py`, where a parse is one step inside an assertion about
//! recovery or about an HTTP response. So the rows are the whole oracle here,
//! and they were chosen to cover the parts a reader would otherwise guess at:
//!
//! * the Python dict repr (`{'reply': 'x', 'tool_calls': [],}`) is a document
//!   and a bare sentence is not, which is the difference between
//!   `parse_structured_candidate` answering `None` and answering a dict;
//! * `response` is an alias only when `reply` is absent, and `replies` wins over
//!   `reply` only after its segments survive normalisation;
//! * a document keeps keys it was not told about (`extra_key`), so the Rust
//!   compares whole values rather than the four keys it knows;
//! * `settle_prose_tool_calls` moves a `{tool_calls}…{/tool_calls}` envelope
//!   out of the reply, but leaves an unpaired `{tool_calls}` behind as the
//!   whitespace it was, having only warned about it.
//!
//! The `warnings` each settle row carries are *not* asserted. They are the
//! payload of the reference's `logger.warning`, and the Rust emits the same
//! message through `tracing::warn!`, which no test can read without a
//! subscriber; the two rows that warn (the unpaired `{tool_calls}` pair) are
//! asserted on the document they leave behind, which is what the caller sees.

mod common;

use common::{cases, expected_text, fixture, input_text, value};
use serde_json::Value;
use shed_chat::{
    parse_model_json_content, parse_model_reply_document, parse_structured_candidate,
    reply_without_prose_tool_calls, settle_prose_tool_calls, ReplyDocument,
};

/// Every `{input, expected}` row of a document section, asserted against the
/// function that produced it.
///
/// A `null` input is fed as `""`. Both source functions open
/// `str(content or "")`, and each section also carries a `""` row whose
/// expectation is the same, so the two spellings are one assertion once the
/// Rust takes a `&str`.
fn check_document_section(section: &str, parse: fn(&str) -> ReplyDocument) {
    let doc = fixture("document.json");
    for case in cases(&doc, section) {
        let got = parse(input_text(case, "input")).into_value();
        assert_eq!(
            got,
            *value(case, "expected"),
            "{section}({:?})",
            input_text(case, "input")
        );
    }
}

#[test]
fn fixture_parse_model_reply_document() {
    check_document_section("parse_model_reply_document", parse_model_reply_document);
}

#[test]
fn fixture_parse_model_json_content() {
    check_document_section("parse_model_json_content", parse_model_json_content);
}

#[test]
fn fixture_parse_structured_candidate() {
    let doc = fixture("document.json");
    for case in cases(&doc, "parse_structured_candidate") {
        let got = parse_structured_candidate(input_text(case, "input"));
        let expected = value(case, "expected");
        match (&got, expected) {
            // `None` is the answer that says "this was never a document", and
            // it is not the same as the empty document the two parsers above
            // answer instead.
            (None, Value::Null) => {}
            (Some(document), Value::Object(_)) => {
                assert_eq!(
                    document.as_value(),
                    expected,
                    "parse_structured_candidate({:?})",
                    input_text(case, "input")
                );
            }
            _ => panic!(
                "parse_structured_candidate({:?}) gave {got:?}, expected {expected}",
                input_text(case, "input")
            ),
        }
    }
}

#[test]
fn fixture_settle_prose_tool_calls() {
    let doc = fixture("document.json");
    for case in cases(&doc, "settle_prose_tool_calls") {
        // The source is handed the dict a parse already produced, so the
        // document is built from the value whole — keys the parser would
        // never have made included.
        let mut document = ReplyDocument::from_value(value(case, "input").clone());
        settle_prose_tool_calls(&mut document);
        assert_eq!(
            document.as_value(),
            value(case, "expected"),
            "settle_prose_tool_calls({})",
            value(case, "input")
        );
    }
}

#[test]
fn fixture_reply_without_prose_tool_calls() {
    let doc = fixture("document.json");
    for case in cases(&doc, "reply_without_prose_tool_calls") {
        assert_eq!(
            reply_without_prose_tool_calls(input_text(case, "input")),
            expected_text(case, "expected"),
            "reply_without_prose_tool_calls({:?})",
            input_text(case, "input")
        );
    }
}

/// The same rows read back through the three accessors every reader in the
/// source uses, rather than through the value as a whole.
///
/// `reply()` is `str(document.get("reply") or "")`, `tool_calls()` is the list
/// or nothing, and `reply_segments()` is present only when the parser stored
/// it — which the source does only for more than one beat, so its absence is
/// the statement "one bubble".
#[test]
fn fixture_document_readers() {
    let doc = fixture("document.json");
    for case in cases(&doc, "parse_model_reply_document") {
        let text = input_text(case, "input");
        let document = parse_model_reply_document(text);
        let expected = value(case, "expected");

        let reply = expected["reply"]
            .as_str()
            .unwrap_or_else(|| panic!("expected reply of {text:?} is not a string: {expected}"));
        assert_eq!(document.reply(), reply, "reply() of {text:?}");

        match &expected["tool_calls"] {
            Value::Array(calls) => {
                assert_eq!(document.tool_calls(), *calls, "tool_calls() of {text:?}")
            }
            // A document whose calls are `null` — the source keeps the `null`,
            // and every reader treats it as no calls.
            _ => assert!(document.tool_calls().is_empty(), "tool_calls() of {text:?}"),
        }

        match &expected["reply_segments"] {
            Value::Array(segments) => assert_eq!(
                document.reply_segments(),
                segments
                    .iter()
                    .map(|segment| segment.as_str().unwrap_or_else(|| {
                        panic!("expected segments of {text:?} hold a non-string: {expected}")
                    }))
                    .collect::<Vec<&str>>(),
                "reply_segments() of {text:?}"
            ),
            Value::Null => assert!(
                document.reply_segments().is_empty(),
                "reply_segments() of {text:?}"
            ),
            _ => panic!("expected reply_segments of {text:?} is not a list: {expected}"),
        }
    }
}

/// `REPLY_DOCUMENT_KEYS`, pinned through the one public behaviour it controls.
///
/// The tuple is private in both languages, and what it decides is a yes/no: a
/// dict carrying one of these keys is a reply document, one carrying none of
/// them is not. So each captured key is fed as the only key of an object, and
/// a key outside the list is fed beside it as the control.
#[test]
fn fixture_reply_document_keys() {
    let doc = fixture("document.json");
    let keys = doc["reply_document_keys"].as_array().unwrap_or_else(|| {
        panic!(
            "reply_document_keys is not a list: {}",
            doc["reply_document_keys"]
        )
    });
    for key in keys {
        let name = key
            .as_str()
            .unwrap_or_else(|| panic!("key {key} is not a string"));
        let candidate = format!(r#"{{"{name}": "a value"}}"#);
        assert!(
            parse_structured_candidate(&candidate).is_some(),
            "{candidate} should be a document"
        );
    }
    // The control, which is also row 4 of the candidate section: a dict the
    // parser has no use for is not a document, and `literal_eval` reading a
    // brace-bearing sentence is exactly what that stops.
    assert!(parse_structured_candidate(r#"{"other": "a value"}"#).is_none());
}
