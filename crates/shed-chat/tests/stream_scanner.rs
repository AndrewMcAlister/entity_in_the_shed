//! The reply stream scanner: the reply text pulled out of the JSON as it lands.
//!
//! A port of `tests/test_reply_stream_scanner.py`: all 20 of its tests, names
//! kept verbatim (they are unique in the Python file), plus every case of
//! `tests/fixtures/stream_scanner.json` — the reference app's own output,
//! captured by `tools/capture_reply_fixtures.py` for the 28 documents it feeds
//! the scanner, each in four chunk sequences.
//!
//! Three things the Python never has to say and this file does:
//!
//! * `scan(list(document))` in the reference is the document one character at
//!   a time; [`chars`] says so rather than feeding a whole string.
//! * One reference test finishes with `text.encode('utf-8')`, which is the
//!   assertion that a lone surrogate did not survive. A Rust `String` cannot
//!   hold an unencodable code point in the first place, so the equality
//!   against `U+FFFD` above it is that same assertion, spelled in the only way
//!   this language has.
//! * The fixture's `same_as_whole` is the capture script comparing a sequence's
//!   text against the one-chunk scan of the same document, so the port
//!   recomputes that scan rather than trusting the label of the first sequence.

mod common;

use serde_json::Value;
use shed_chat::JsonReplyStreamScanner;

use common::{cases, chunks, expected_bool, expected_text, field, fixture, text_list, value};

// ---------------------------------------------------------------------------
// The driver of the reference file, transcribed.
// ---------------------------------------------------------------------------

/// Feed chunks through the scanner and collect what it emits, then flush.
///
/// The reference skips an emission before appending it, which cannot change a
/// join on the empty string.
fn scan(pieces: &[Option<&str>]) -> (String, JsonReplyStreamScanner) {
    let mut scanner = JsonReplyStreamScanner::new();
    let mut text = String::new();
    for piece in pieces {
        text.push_str(&scanner.feed(*piece));
    }
    text.push_str(&scanner.finish());
    (text, scanner)
}

fn scan_one(document: &str) -> (String, JsonReplyStreamScanner) {
    scan(&[Some(document)])
}

/// `list(document)` in the reference: the text as one-character chunks.
fn chars(text: &str) -> Vec<Option<&str>> {
    text.char_indices()
        .map(|(start, ch)| Some(&text[start..start + ch.len_utf8()]))
        .collect()
}

fn count(doc: &Value, key: &str) -> usize {
    usize::try_from(
        doc[key]
            .as_u64()
            .unwrap_or_else(|| panic!("count {key} is not a number: {doc}")),
    )
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// tests/test_reply_stream_scanner.py
// ---------------------------------------------------------------------------

#[test]
fn whole_document_in_one_chunk() {
    let (text, scanner) = scan_one(r#"{"reply": "Hello there.", "tool_calls": []}"#);

    assert_eq!(text, "Hello there.");
    assert_eq!(scanner.reply(), "Hello there.");
    assert!(scanner.is_done());
    assert!(!scanner.is_aborted());
}

#[test]
fn character_by_character_is_the_same_as_one_chunk() {
    let document = r#"{"reply": "One two three.", "tool_calls": []}"#;
    let (whole, _) = scan_one(document);
    let (split, scanner) = scan(&chars(document));

    assert_eq!(split, whole);
    assert!(scanner.is_done());
}

#[test]
fn response_key_is_accepted_like_the_parser_does() {
    let (text, scanner) = scan_one(r#"{"response": "Still fine."}"#);

    assert_eq!(text, "Still fine.");
    assert!(scanner.is_done());
}

#[test]
fn whitespace_inside_the_object_is_tolerated() {
    let (text, scanner) = scan_one("{  \"reply\"\n\t:\n  \"Spaced out.\"  }");

    assert_eq!(text, "Spaced out.");
    assert!(scanner.is_done());
}

#[test]
fn fenced_json_is_tolerated() {
    let (text, scanner) = scan_one("```json\n{\"reply\": \"Fenced.\"}\n```");

    assert_eq!(text, "Fenced.");
    assert!(scanner.is_done());
}

#[test]
fn escapes_are_decoded() {
    let (text, scanner) = scan_one(r#"{"reply": "Line\nbreak \"quoted\" tab\tend."}"#);

    assert_eq!(text, "Line\nbreak \"quoted\" tab\tend.");
    assert!(scanner.is_done());
}

#[test]
fn escape_split_across_chunks() {
    let (text, scanner) = scan(&[Some(r#"{"reply": "First "#), Some(r#"\n second."}"#)]);

    assert_eq!(text, "First \n second.");
    assert!(scanner.is_done());
}

#[test]
fn unicode_escape_split_mid_digits() {
    let (text, scanner) = scan(&[Some(r#"{"reply": "caf\u00"#), Some("e9 done.\"}")]);

    assert_eq!(text, "café done.");
    assert!(scanner.is_done());
}

#[test]
fn surrogate_pair_split_across_chunks_forms_one_code_point() {
    let (text, scanner) = scan(&[Some(r#"{"reply": "wave \uD83D"#), Some(r#"\uDE00 end."}"#)]);

    assert_eq!(text, "wave \u{1F600} end.");
    assert!(scanner.is_done());
}

#[test]
fn lone_high_surrogate_becomes_a_replacement_character() {
    // A lone surrogate cannot be encoded, and this text ends up inside the
    // JSON response the browser is polling, so it must not survive as one.
    let (text, scanner) = scan_one(r#"{"reply": "broken \uD83D end."}"#);

    assert_eq!(text, "broken \u{FFFD} end.");
    assert!(scanner.is_done());
}

#[test]
fn trailing_lone_surrogate_is_flushed_at_end_of_stream() {
    let (text, scanner) = scan_one(r#"{"reply": "unfinished \uD83D"#);

    assert_eq!(text, "unfinished \u{FFFD}");
    assert!(!scanner.is_done());
}

#[test]
fn reply_stops_at_the_closing_quote() {
    let (text, scanner) = scan_one(r#"{"reply": "Stop here.", "tool_calls": [{"tool": "x"}]}"#);

    assert_eq!(text, "Stop here.");
    assert!(scanner.is_done());
    // Everything after the reply string is none of this scanner's business.
    assert!(!scanner.reply().contains("tool_calls"));
}

#[test]
fn tool_calls_first_aborts_without_emitting() {
    let (text, scanner) = scan_one(r#"{"tool_calls": [{"tool": "search_web"}], "reply": "Late."}"#);

    assert_eq!(text, "");
    assert!(scanner.is_aborted());
    assert!(!scanner.is_done());
}

#[test]
fn prose_preamble_aborts() {
    let (text, scanner) = scan_one("Sure, here is my answer: {\"reply\": \"Ignored.\"}");

    assert_eq!(text, "");
    assert!(scanner.is_aborted());
}

#[test]
fn bare_text_aborts() {
    let (text, scanner) = scan_one("The answer is forty-two.");

    assert_eq!(text, "");
    assert!(scanner.is_aborted());
}

#[test]
fn non_string_reply_aborts() {
    let (text, scanner) = scan_one(r#"{"reply": 42}"#);

    assert_eq!(text, "");
    assert!(scanner.is_aborted());
}

#[test]
fn unbounded_prose_preamble_aborts_instead_of_growing() {
    let overlong = "x".repeat(JsonReplyStreamScanner::MAX_PREAMBLE_CHARS + 10);
    let (text, scanner) = scan_one(&overlong);

    assert_eq!(text, "");
    assert!(scanner.is_aborted());
}

#[test]
fn abort_is_terminal() {
    let (_, mut scanner) = scan_one("nonsense");
    assert_eq!(scanner.feed(Some(r#"{"reply": "Too late."}"#)), "");
    assert_eq!(scanner.reply(), "");
    assert!(scanner.is_aborted());
}

#[test]
fn unknown_escape_keeps_the_character() {
    let (text, scanner) = scan_one(r#"{"reply": "a \q b."}"#);

    assert_eq!(text, "a q b.");
    assert!(scanner.is_done());
}

#[test]
fn empty_and_none_chunks_are_ignored() {
    let mut scanner = JsonReplyStreamScanner::new();
    assert_eq!(scanner.feed(Some("")), "");
    assert_eq!(scanner.feed(None), "");
    assert_eq!(scanner.reply(), "");
    assert!(!scanner.is_aborted());
}

// ---------------------------------------------------------------------------
// stream_scanner.json — every case the capture script recorded.
//
// The tests above are the reference app's own assertions, transcribed. These
// walk the golden file: 28 documents — the ones above plus the shapes the
// Python test file never tried, a `"Reply"` of the wrong case, a `rep` that is
// only the prefix of a key, a fence with a space in it, a reply that ends with
// a backslash — each fed whole, one character at a time, and at two seeded
// cuts.
// ---------------------------------------------------------------------------

#[test]
fn fixture_constants() {
    let doc = fixture("stream_scanner.json");
    assert_eq!(
        JsonReplyStreamScanner::MAX_PREAMBLE_CHARS,
        count(&doc, "max_preamble_chars")
    );
    assert_eq!(
        JsonReplyStreamScanner::KEYS.to_vec(),
        text_list(&doc, "keys")
    );
}

#[test]
fn fixture_stream_scanner_sequences() {
    let doc = fixture("stream_scanner.json");
    for (index, case) in cases(&doc, "cases").iter().enumerate() {
        // The capture compares every sequence against the same document fed in
        // one piece, which is what `same_as_whole` means.
        let whole = scan_one(field(case, "document")).0;
        for sequence in cases(case, "sequences") {
            let label = format!("case {index} {}", field(sequence, "label"));
            let (text, scanner) = scan(&chunks(sequence, "chunks"));
            assert_eq!(text, expected_text(sequence, "text"), "{label} text");
            assert_eq!(
                scanner.reply(),
                expected_text(sequence, "reply"),
                "{label} reply"
            );
            assert_eq!(
                scanner.is_done(),
                expected_bool(sequence, "done"),
                "{label} done"
            );
            assert_eq!(
                scanner.is_aborted(),
                expected_bool(sequence, "aborted"),
                "{label} aborted"
            );
            assert_eq!(
                text == whole,
                expected_bool(sequence, "same_as_whole"),
                "{label} same_as_whole"
            );
        }
    }
}

#[test]
fn fixture_empty_and_none_chunks() {
    let doc = fixture("stream_scanner.json");
    let ignored = value(&doc, "empty_and_none_chunks");
    let mut scanner = JsonReplyStreamScanner::new();

    assert_eq!(scanner.feed(Some("")), expected_text(ignored, "feed_empty"));
    assert_eq!(scanner.feed(None), expected_text(ignored, "feed_none"));
    assert_eq!(scanner.reply(), expected_text(ignored, "reply"));
    assert!(!expected_bool(ignored, "aborted"));
}

#[test]
fn fixture_abort_is_terminal() {
    let doc = fixture("stream_scanner.json");
    let terminal = value(&doc, "abort_is_terminal");
    let (text, scanner) = scan(&chunks(terminal, "chunks"));

    assert_eq!(text, expected_text(terminal, "text"));
    assert_eq!(scanner.reply(), expected_text(terminal, "reply"));
    assert_eq!(scanner.is_done(), expected_bool(terminal, "done"));
    assert_eq!(scanner.is_aborted(), expected_bool(terminal, "aborted"));

    // The capture aborts a second scanner the same way — without flushing it —
    // and records what a later feed is left able to say. That feed is the
    // literal the capture script types, not a fixture field.
    let mut later = JsonReplyStreamScanner::new();
    for piece in chunks(terminal, "chunks") {
        later.feed(piece);
    }
    assert_eq!(
        later.feed(Some(r#"{"reply": "Too late."}"#)),
        expected_text(terminal, "later_feed")
    );
    assert_eq!(later.reply(), expected_text(terminal, "later_reply"));
}
