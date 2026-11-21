//! Speaking a reply while it is still being written, and what is left to say.
//!
//! A port of `tests/test_reply_stream_publisher.py`: all 17 of its tests, names
//! kept verbatim (they are unique in the Python file), plus every case of
//! `tests/fixtures/publisher.json` — the reference app's own output for ten
//! replies in six chunk sequences each, the five hand-built streams the capture
//! script records beside them, and the fifteen reply/spoken pairs the two
//! after-the-stream helpers are given.
//!
//! Three things the Python never has to say and this file does:
//!
//! * The reference appends to a plain list and hands `list.append` to the
//!   publisher. [`PublishFn`] is owned, so the list is shared through a
//!   reference count instead — see [`recorder`].
//! * `json.dumps({"reply": …})` decides where a chunk cut lands, so
//!   [`document`] reproduces its exact spacing rather than serde_json's
//!   tighter one; one reference test cuts the document at its middle and asserts
//!   what had been published by then.
//! * The fixture's `spoken` is what `finish()` returns and its `published` is
//!   what the callback received. They are equal for every row captured here, and
//!   the port asserts both separately because the two are not the same thing.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;
use shed_chat::{
    split_reply_after_stream, streamed_prefix_length, strip_streamed_prefix, PublishFn,
    ReplyStreamPublisher,
};

use common::{
    cases, chunks, expected_bool, expected_text, field, fixture, input_text, text_list, value,
};

// ---------------------------------------------------------------------------
// The drivers of the reference file, transcribed.
// ---------------------------------------------------------------------------

/// The list the reference passes as `published`, and the callback that fills it.
///
/// `PublishFn` owns its closure, so the test reaches the same list through a
/// shared count rather than the borrow `published.append` gets for free.
fn recorder() -> (Rc<RefCell<Vec<String>>>, PublishFn) {
    let published = Rc::new(RefCell::new(Vec::new()));
    let callback = {
        let published = Rc::clone(&published);
        Box::new(move |sentence: &str| published.borrow_mut().push(sentence.to_owned()))
    };
    (published, callback)
}

fn recorded(published: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    published.borrow().clone()
}

/// Feed a reply through the publisher the way the token stream would, then close
/// the stream — the reference's `publish`, which always finishes.
fn publish(pieces: &[Option<&str>]) -> (Vec<String>, String, ReplyStreamPublisher) {
    let (published, callback) = recorder();
    let mut publisher = ReplyStreamPublisher::new(callback);
    for piece in pieces {
        publisher.feed(*piece);
    }
    let spoken = publisher.finish();
    (recorded(&published), spoken, publisher)
}

/// Publish a reply the way it arrives: JSON wrapper and all.
fn stream_reply(reply: &str, chunk_size: usize) -> (Vec<String>, String, ReplyStreamPublisher) {
    let wrapper = document(reply);
    let pieces: Vec<Option<&str>> = slices(&wrapper, chunk_size).into_iter().map(Some).collect();
    publish(&pieces)
}

/// `json.dumps({"reply": reply, "tool_calls": []})`.
///
/// The spacing matters: `json.dumps` puts a space after each separator where
/// serde_json puts none, and the reference cuts this string by character count.
/// Every reply fed here is ASCII, so serde_json's escaping and Python's agree.
fn document(reply: &str) -> String {
    format!("{{\"reply\": {}, \"tool_calls\": []}}", Value::from(reply))
}

/// `text[index:index + size]` for a whole string: characters, not bytes.
fn slices(text: &str, size: usize) -> Vec<&str> {
    let bounds: Vec<(usize, usize)> = text
        .char_indices()
        .map(|(start, character)| (start, start + character.len_utf8()))
        .collect();
    bounds
        .chunks(size.max(1))
        .map(|group| &text[group[0].0..group[group.len() - 1].1])
        .collect()
}

/// `text[:count]`, again by character.
fn prefix(text: &str, count: usize) -> &str {
    text.char_indices()
        .nth(count)
        .map_or(text, |(start, _)| &text[..start])
}

// ---------------------------------------------------------------------------
// tests/test_reply_stream_publisher.py
// ---------------------------------------------------------------------------

#[test]
fn publishes_finished_sentences_in_order() {
    let (published, spoken, _) = stream_reply(
        "Let me check that for you. It is forty-two. That is all.",
        5,
    );

    assert_eq!(
        published,
        [
            "Let me check that for you.",
            "It is forty-two.",
            "That is all."
        ]
    );
    assert_eq!(spoken, published.join(" "));
}

#[test]
fn publishes_before_the_document_is_complete() {
    // The point of the whole exercise: sentences must arrive early.
    let (published, callback) = recorder();
    let mut publisher = ReplyStreamPublisher::new(callback);
    let wrapper = document("First sentence arrives now. Second one is later on.");
    let half = wrapper.chars().count() / 2;

    publisher.feed(Some(prefix(&wrapper, half)));

    // Not finished: only what the stream had already completed is out.
    assert_eq!(recorded(&published), ["First sentence arrives now."]);
}

#[test]
fn an_aborted_layout_publishes_nothing() {
    let (published, _, mut publisher) =
        publish(&[Some(r#"{"tool_calls": [], "reply": "Too late."}"#)]);

    assert_eq!(published, Vec::<String>::new());
    assert!(publisher.aborted);
    assert_eq!(publisher.finish(), "");
}

#[test]
fn publishing_stops_after_an_abort() {
    let (published, callback) = recorder();
    let mut publisher = ReplyStreamPublisher::new(callback);
    publisher.feed(Some(r#"{"tool_calls": []"#));
    publisher.feed(Some(r#", "reply": "Never spoken."}"#));

    assert_eq!(recorded(&published), Vec::<String>::new());
}

#[test]
fn the_remainder_after_the_stream_is_reported() {
    let (published, spoken, _) = stream_reply("One two three four. Five six seven eight.", 5);

    assert_eq!(published, ["One two three four.", "Five six seven eight."]);
    assert_eq!(spoken, "One two three four. Five six seven eight.");
}

#[test]
fn a_missing_publish_callback_is_tolerated() {
    let mut publisher = ReplyStreamPublisher::without_publish();
    publisher.feed(Some(&document("Spoken to nobody. But counted.")));

    assert_eq!(publisher.published, 1);
    assert_eq!(publisher.spoken_text(), "Spoken to nobody.");
}

#[test]
fn published_count_matches_the_list() {
    let (published, _, publisher) = stream_reply("A sentence here. Another sentence there.", 5);

    assert_eq!(publisher.published, published.len());
}

#[test]
fn remaining_is_the_part_not_yet_spoken() {
    let (remaining, replaced) = split_reply_after_stream(
        "Let me check that for you. It is forty-two.",
        "Let me check that for you.",
    );

    assert_eq!(remaining, "It is forty-two.");
    assert!(!replaced);
}

#[test]
fn nothing_remaining_when_the_whole_reply_was_spoken() {
    let (remaining, replaced) =
        split_reply_after_stream("That is the whole thing.", "That is the whole thing.");

    assert_eq!(remaining, "");
    assert!(!replaced);
}

#[test]
fn nothing_spoken_means_the_whole_reply_remains() {
    let (remaining, replaced) = split_reply_after_stream("Whole reply here.", "");

    assert_eq!(remaining, "Whole reply here.");
    assert!(!replaced);
}

#[test]
fn whitespace_differences_do_not_count_as_a_replacement() {
    // The splitter strips each sentence, so spacing legitimately differs.
    let (remaining, replaced) = split_reply_after_stream(
        "One two three.   Four five six.\nSeven eight nine.",
        "One two three. Four five six.",
    );

    assert_eq!(remaining, "Seven eight nine.");
    assert!(!replaced);
}

#[test]
fn a_rewritten_reply_is_reported_as_replaced() {
    let (remaining, replaced) = split_reply_after_stream(
        "Something entirely different was said instead.",
        "The beginning the user actually heard.",
    );

    assert_eq!(remaining, "Something entirely different was said instead.");
    assert!(replaced);
}

#[test]
fn a_replacement_is_not_reported_when_nothing_was_spoken() {
    let (_, replaced) = split_reply_after_stream("Anything at all.", "");

    assert!(!replaced);
}

#[test]
fn a_repeated_preface_is_dropped() {
    assert_eq!(
        strip_streamed_prefix(
            "Let me check that now. The market is up three percent.",
            "Let me check that now.",
        ),
        "The market is up three percent."
    );
}

#[test]
fn a_different_follow_up_is_left_alone() {
    assert_eq!(
        strip_streamed_prefix("The market is up three percent.", "Let me check that now.",),
        "The market is up three percent."
    );
}

#[test]
fn nothing_spoken_leaves_the_reply_alone() {
    assert_eq!(
        strip_streamed_prefix("Unchanged reply.", ""),
        "Unchanged reply."
    );
}

#[test]
fn a_reply_that_is_only_the_preface_becomes_empty() {
    assert_eq!(
        strip_streamed_prefix("Let me check that now.", "Let me check that now."),
        ""
    );
}

// ---------------------------------------------------------------------------
// publisher.json — every stream the capture script recorded.
//
// The tests above are the reference app's own assertions, transcribed. These
// walk the golden file: the ten replies the tests above build, each streamed at
// four fixed chunk sizes and two seeded cuts, then the five hand-built streams a
// reply can never produce (the aborts, and a document that never closed), then
// the fifteen reply/spoken pairs the after-the-stream helpers are asked about.
// ---------------------------------------------------------------------------

/// One recorded stream: what the callback got, what `finish()` reported, and
/// what the publisher says about itself afterwards.
fn assert_stream(label: &str, sequence: &Value) {
    let (published, spoken, publisher) = publish(&chunks(sequence, "chunks"));

    assert_eq!(
        published,
        text_list(sequence, "published"),
        "{label} published"
    );
    assert_eq!(spoken, expected_text(sequence, "spoken"), "{label} spoken");
    assert_eq!(
        publisher.aborted,
        expected_bool(sequence, "aborted"),
        "{label} aborted"
    );
    assert_eq!(
        publisher.published,
        usize::try_from(
            value(sequence, "count")
                .as_u64()
                .unwrap_or_else(|| panic!("{label} count is not a number: {sequence}"))
        )
        .unwrap_or_default(),
        "{label} count"
    );
    assert_eq!(
        publisher.spoken_text(),
        expected_text(sequence, "spoken_text"),
        "{label} spoken_text"
    );
}

#[test]
fn fixture_publisher_cases() {
    let doc = fixture("publisher.json");
    for (index, case) in cases(&doc, "cases").iter().enumerate() {
        // The wrapper the capture streamed, which is also what [`document`]
        // builds for the tests above — worth pinning, because a wrapper of a
        // different length cuts at a different place.
        assert_eq!(
            document(input_text(case, "reply")),
            field(case, "document"),
            "case {index} document"
        );
        for sequence in cases(case, "sequences") {
            assert_stream(
                &format!("case {index} {}", field(sequence, "label")),
                sequence,
            );
        }
    }
}

#[test]
fn fixture_publisher_hand_built_streams() {
    let doc = fixture("publisher.json");
    for row in cases(&doc, "hand_built") {
        assert_stream(&format!("hand built {}", field(row, "label")), row);
    }
}

#[test]
fn fixture_publisher_streamed() {
    let doc = fixture("publisher.json");
    for (index, row) in cases(&doc, "streamed").iter().enumerate() {
        let label = format!("streamed row {index}");
        let reply = input_text(row, "reply");
        let spoken = input_text(row, "spoken");

        let want_length = match value(row, "prefix_length") {
            Value::Null => None,
            other => Some(
                usize::try_from(
                    other
                        .as_u64()
                        .unwrap_or_else(|| panic!("{label} prefix_length is not a number: {row}")),
                )
                .unwrap_or_default(),
            ),
        };
        assert_eq!(
            streamed_prefix_length(reply, spoken),
            want_length,
            "{label} prefix_length"
        );

        let pair = value(row, "split");
        let want_remaining = pair[0]
            .as_str()
            .unwrap_or_else(|| panic!("{label} split is not a pair: {pair}"));
        let want_replaced = pair[1]
            .as_bool()
            .unwrap_or_else(|| panic!("{label} split is not a pair: {pair}"));
        let (remaining, replaced) = split_reply_after_stream(reply, spoken);
        assert_eq!(remaining, want_remaining, "{label} split remaining");
        assert_eq!(replaced, want_replaced, "{label} split replaced");

        assert_eq!(
            strip_streamed_prefix(reply, spoken),
            expected_text(row, "stripped"),
            "{label} stripped"
        );
    }
}
