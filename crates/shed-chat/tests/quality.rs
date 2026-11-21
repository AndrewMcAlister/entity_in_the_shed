//! The reply-quality judgements, the prose tool-call envelope, and the
//! duplicate-event guard.
//!
//! A port of `tests/test_reply_utils.py`: all 78 of its tests, with the names
//! kept verbatim wherever the flat Rust namespace allows (three names —
//! `test_blank_and_none`, `test_normal_reply`, `test_blank` — appear in more
//! than one Python class and are prefixed with their class here), plus every
//! case of `tests/fixtures/quality.json`, which is the reference app's own
//! output captured by `tools/capture_reply_fixtures.py`.
//!
//! Two things the Python never has to say and this file does:
//!
//! * Half of these guards take `Optional[str]` in the source and `&str` here.
//!   That is honest rather than convenient: every `null` in the fixture was run
//!   against the reference next to its `""` twin before this file was written,
//!   and the two answers were identical for all fourteen functions, so
//!   [`common::input_text`] reads the fixture's `null` as the empty string.
//! * `reply._is_duplicate_event(event_id, lock, recent)` owns its memory here.
//!   The `deque(maxlen=…)` and the lock the caller passes it become
//!   [`RecentEvents`] and its `&mut self`.

mod common;

use serde_json::{json, Value};
use shed_chat::{
    extract_reply_segments, latest_user_message_text, looks_incomplete,
    looks_like_action_teaser_reply, looks_like_generic_handoff_reply,
    looks_like_mctrader_action_request, looks_like_recovery_meta_reply, looks_like_task_stub_reply,
    looks_out_of_character_reply, looks_unusable_reply, merge_completed_reply,
    strip_tool_call_envelope, tool_calls_in_envelope, RecentEvents, REPLY_SEGMENT_MAX,
    TEASER_MAX_WORDS,
};

use common::{cases, expected_bool, expected_text, fixture, input_text, text_list, value};

/// The array a case field holds; the fixture spells a missing list `null`.
fn array<'a>(case: &'a Value, key: &str) -> &'a [Value] {
    match &case[key] {
        Value::Array(items) => items,
        Value::Null => &[],
        other => panic!("case field {key} is not an array: {other}"),
    }
}

/// A value that is itself a list, for the inputs the Python is handed directly.
fn items(value: &Value) -> &[Value] {
    match value {
        Value::Array(items) => items,
        Value::Null => &[],
        other => panic!("expected a list, found {other}"),
    }
}

/// The same list read as booleans, for the `expected` of a sequence case.
fn bools(case: &Value, key: &str) -> Vec<bool> {
    array(case, key)
        .iter()
        .map(|item| {
            item.as_bool()
                .unwrap_or_else(|| panic!("case field {key} holds a non-bool: {case}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// TestLooksLikeActionTeaserReply
// ---------------------------------------------------------------------------

#[test]
fn teaser_phrases() {
    assert!(looks_like_action_teaser_reply("Let me fetch that."));
    assert!(looks_like_action_teaser_reply("I'll check on it."));
    assert!(looks_like_action_teaser_reply("Fetching the data now."));
    assert!(looks_like_action_teaser_reply("Checking..."));
}

#[test]
fn the_reported_failure() {
    // The transcript that started this: the entity promised to look and never
    // followed up, and the reply was persisted as the final answer. None of the
    // retrieval-verb patterns match it, because "take" is not a retrieval verb
    // — hence the separate promise-of-action axis.
    assert!(looks_like_action_teaser_reply("Let me take a closer look."));
}

#[test]
fn promise_of_a_look() {
    for text in [
        "Let me have a look.",
        "Let me look into that.",
        "Let me see what I can find.",
        "I'll take a closer look.",
        "I will have another look.",
        "Taking a closer look now.",
        "Looking into this now.",
        "Let me dig into the numbers.",
        "Let me investigate.",
        "I'll go over it.",
    ] {
        assert!(looks_like_action_teaser_reply(text), "{text}");
    }
}

#[test]
fn courtesy_is_not_a_teaser() {
    // "let me know" offers the user something; it does not announce work the
    // entity owes them. Matching it would send ordinary sign-offs into
    // recovery, and at agent.py:resolve_followup_action_request a match means
    // the prior request is re-issued.
    assert!(!looks_like_action_teaser_reply(
        "Let me know if you need anything else."
    ));
    assert!(!looks_like_action_teaser_reply(
        "Let me know how you get on."
    ));
    assert!(!looks_like_action_teaser_reply(
        "I'll let you know when it is done."
    ));
}

#[test]
fn long_answer_containing_a_teaser_phrase_is_not_a_teaser() {
    // The length guard. A real answer can contain "checking" without
    // announcing anything, and treating it as a teaser would replace a
    // complete reply with a fresh one.
    let long_answer = "The position is up two point four percent on the day and the margin \
                       sits at thirty percent, with the stop still where we left it, so I am \
                       checking nothing further and would leave it be for now, as the trend \
                       has held across every session this week and there is no reason to act.";
    assert!(long_answer.split_whitespace().count() > TEASER_MAX_WORDS);
    assert!(!looks_like_action_teaser_reply(long_answer));
}

#[test]
fn boundary_is_the_word_count() {
    // Just under the limit still counts; just over does not. Asserted on the
    // constant so the two cannot drift apart. "Let me check." is three words,
    // so the filler makes up the remainder.
    let filler = std::iter::repeat_n("ok", TEASER_MAX_WORDS - 3)
        .collect::<Vec<_>>()
        .join(" ");
    let short = format!("Let me check. {filler}");
    assert_eq!(short.split_whitespace().count(), TEASER_MAX_WORDS);
    assert!(looks_like_action_teaser_reply(&short));

    let long = format!("{short} ok");
    assert!(long.split_whitespace().count() > TEASER_MAX_WORDS);
    assert!(!looks_like_action_teaser_reply(&long));
}

#[test]
fn plain_reply() {
    assert!(!looks_like_action_teaser_reply("Here is the data."));
    assert!(!looks_like_action_teaser_reply(
        "The weather is nice today."
    ));
}

#[test]
fn teaser_blank_and_none() {
    assert!(!looks_like_action_teaser_reply(""));
    // The Python's `None`, which the reference reads as `""`.
    assert!(!looks_like_action_teaser_reply(""));
}

// ---------------------------------------------------------------------------
// TestLooksLikeMctraderActionRequest
// ---------------------------------------------------------------------------

#[test]
fn context_and_intent() {
    assert!(looks_like_mctrader_action_request(
        "show my mctrader balance"
    ));
    assert!(looks_like_mctrader_action_request(
        "check my broker position"
    ));
    assert!(looks_like_mctrader_action_request("start trader now"));
}

#[test]
fn context_only() {
    assert!(!looks_like_mctrader_action_request("what is mctrader"));
}

#[test]
fn intent_only() {
    assert!(!looks_like_mctrader_action_request("check my position"));
}

#[test]
fn mctrader_blank_and_none() {
    assert!(!looks_like_mctrader_action_request(""));
    assert!(!looks_like_mctrader_action_request(""));
}

// ---------------------------------------------------------------------------
// TestLooksIncomplete
// ---------------------------------------------------------------------------

#[test]
fn incomplete_blank_and_none() {
    assert!(looks_incomplete("", None));
    assert!(looks_incomplete("", None));
}

#[test]
fn done_reason_length() {
    assert!(looks_incomplete(
        "Any text at all",
        Some(&json!({ "_meta": { "done_reason": "length" } })),
    ));
}

#[test]
fn done_reason_stop_not_incomplete() {
    assert!(!looks_incomplete(
        "Done here.",
        Some(&json!({ "_meta": { "done_reason": "stop" } })),
    ));
}

#[test]
fn trailing_colon_or_ellipsis() {
    assert!(looks_incomplete("Wait for it:", None));
    assert!(looks_incomplete("Hold on...", None));
}

#[test]
fn long_multiline_without_terminal_punctuation() {
    let text = "This is a fairly long incomplete line\nand another line here now";
    assert!(text.chars().count() >= 60);
    assert!(looks_incomplete(text, None));
}

#[test]
fn poem_teaser_phrase() {
    assert!(looks_incomplete("Here's a poem for you", None));
}

#[test]
fn whitespace_only() {
    assert!(!looks_incomplete("  ", None));
}

#[test]
fn complete_sentence() {
    assert!(!looks_incomplete("The quick brown fox jumped.", None));
}

// ---------------------------------------------------------------------------
// TestLatestUserMessageText
// ---------------------------------------------------------------------------

#[test]
fn returns_last_user_content_stripped() {
    let messages = json!([
        { "role": "user", "content": "  hi there  " },
        { "role": "assistant", "content": "x" },
        { "role": "user", "content": "two" },
    ]);
    assert_eq!(latest_user_message_text(items(&messages)), "two");
}

#[test]
fn no_user_messages() {
    let messages = json!([{ "role": "assistant", "content": "x" }]);
    assert_eq!(latest_user_message_text(items(&messages)), "");
}

#[test]
fn latest_none_and_empty() {
    // The Python passes `None` and `[]`; both are an empty slice here.
    assert_eq!(latest_user_message_text(&[]), "");
    assert_eq!(latest_user_message_text(&[]), "");
}

// ---------------------------------------------------------------------------
// TestLooksLikeTaskStubReply
// ---------------------------------------------------------------------------

#[test]
fn stub_reply() {
    assert!(looks_like_task_stub_reply(
        "write a poem for me",
        "I will write one for you"
    ));
    assert!(looks_like_task_stub_reply(
        "create a song for me",
        "Here is a line for you"
    ));
}

#[test]
fn non_generative_request() {
    assert!(!looks_like_task_stub_reply("what time is it", "It is 3pm."));
}

#[test]
fn multiline_reply_not_stub() {
    assert!(!looks_like_task_stub_reply(
        "write a poem for me",
        "I will write one\nfor you"
    ));
}

#[test]
fn long_reply_not_stub() {
    let long_reply = "I will write one for you right now with some rhymes and verses that flow \
                      really nicely today indeed";
    assert!(long_reply.split_whitespace().count() > 18);
    assert!(!looks_like_task_stub_reply(
        "write a poem for me",
        long_reply
    ));
}

#[test]
fn content_marker_not_stub() {
    assert!(!looks_like_task_stub_reply(
        "write a poem for me",
        "I will write \"a poem\" for you"
    ));
}

#[test]
fn blank_inputs() {
    assert!(!looks_like_task_stub_reply("", "I will"));
    assert!(!looks_like_task_stub_reply("write a poem for me", ""));
}

// ---------------------------------------------------------------------------
// TestMergeCompletedReply
// ---------------------------------------------------------------------------

#[test]
fn completion_starts_with_partial() {
    assert_eq!(merge_completed_reply("Hello", "Hello world"), "Hello world");
    assert_eq!(merge_completed_reply("abc", "abc def"), "abc def");
}

#[test]
fn overlap_join() {
    assert_eq!(
        merge_completed_reply("Hello world", "world is big"),
        "Hello world is big"
    );
    assert_eq!(
        merge_completed_reply("He said", "said hello"),
        "He said hello"
    );
}

#[test]
fn no_overlap_completion_starts_with_capital() {
    assert_eq!(merge_completed_reply("Hi", "There you go"), "There you go");
}

#[test]
fn no_overlap_word_join() {
    assert_eq!(
        merge_completed_reply("The end is", "near"),
        "The end is near"
    );
}

#[test]
fn empty_completion_returns_partial_stripped() {
    assert_eq!(merge_completed_reply("  partial  ", ""), "partial");
}

#[test]
fn empty_partial_returns_completion() {
    assert_eq!(merge_completed_reply("", "Done."), "Done.");
    assert_eq!(merge_completed_reply("", "Done."), "Done.");
}

#[test]
fn equal_replies() {
    assert_eq!(merge_completed_reply("abc", "abc"), "abc");
}

// ---------------------------------------------------------------------------
// TestExtractReplySegments
// ---------------------------------------------------------------------------

#[test]
fn absent_field_yields_no_segments() {
    // The signal the caller reads as "this turn is one reply after all".
    assert_eq!(extract_reply_segments(&Value::Null), Vec::<String>::new());
    assert_eq!(extract_reply_segments(&json!([])), Vec::<String>::new());
    assert_eq!(extract_reply_segments(&json!({})), Vec::<String>::new());
}

#[test]
fn a_single_segment_is_still_returned() {
    // One beat is not the same as no beats: the caller decides what to do with
    // a list of one, and collapsing that here would lose the truth.
    assert_eq!(
        extract_reply_segments(&json!(["Only this."])),
        vec!["Only this.".to_owned()]
    );
}

#[test]
fn strips_and_drops_empties() {
    assert_eq!(
        extract_reply_segments(&json!(["  first  ", "", "   ", "second"])),
        vec!["first", "second"]
    );
}

#[test]
fn drops_non_strings() {
    assert_eq!(
        extract_reply_segments(&json!(["real", null, 7, { "a": 1 }, ["nested"]])),
        vec!["real"]
    );
}

#[test]
fn caps_at_the_maximum() {
    let many: Vec<String> = (0..REPLY_SEGMENT_MAX + 3)
        .map(|index| format!("segment {index}"))
        .collect();
    let segments = extract_reply_segments(&json!(many));
    assert_eq!(segments.len(), REPLY_SEGMENT_MAX);
    assert_eq!(segments[0], "segment 0");
}

#[test]
fn bare_string_is_one_segment() {
    // A model that answers with a plain string rather than an array should not
    // lose its answer.
    assert_eq!(
        extract_reply_segments(&json!("Just the one.")),
        vec!["Just the one."]
    );
}

#[test]
fn tuple_accepted() {
    // A Python tuple arrives as a JSON array.
    assert_eq!(extract_reply_segments(&json!(["a", "b"])), vec!["a", "b"]);
}

#[test]
fn does_not_normalise() {
    // Normalisation lives in agent.py; doing it here would make this module
    // depend on the app it was extracted from.
    assert_eq!(
        extract_reply_segments(&json!(["Hello,world"])),
        vec!["Hello,world"]
    );
}

// ---------------------------------------------------------------------------
// TestLooksUnusableReply
// ---------------------------------------------------------------------------

#[test]
fn known_phrases() {
    assert!(looks_unusable_reply("I could not generate a response"));
    assert!(looks_unusable_reply("i couldn't generate a response"));
    assert!(looks_unusable_reply("I am unable to generate a response."));
}

#[test]
fn unusable_normal_reply() {
    assert!(!looks_unusable_reply("Hello there"));
}

#[test]
fn unusable_blank() {
    assert!(looks_unusable_reply(""));
}

// ---------------------------------------------------------------------------
// TestLooksOutOfCharacterReply
// ---------------------------------------------------------------------------

#[test]
fn forbidden_phrases() {
    assert!(looks_out_of_character_reply(
        "As an AI, I think it is fine."
    ));
    assert!(looks_out_of_character_reply(
        "I do not possess a personal identity."
    ));
}

#[test]
fn out_of_character_normal_reply() {
    assert!(!looks_out_of_character_reply("The weather is nice."));
}

#[test]
fn out_of_character_blank() {
    assert!(!looks_out_of_character_reply(""));
}

// ---------------------------------------------------------------------------
// TestLooksLikeRecoveryMetaReply
// ---------------------------------------------------------------------------

#[test]
fn meta_markers() {
    assert!(looks_like_recovery_meta_reply(
        "There was no content in the previous reply."
    ));
    assert!(looks_like_recovery_meta_reply("empty object"));
}

#[test]
fn recovery_meta_normal_reply() {
    assert!(!looks_like_recovery_meta_reply("All done here."));
}

#[test]
fn recovery_meta_blank() {
    assert!(!looks_like_recovery_meta_reply(""));
}

// ---------------------------------------------------------------------------
// TestLooksLikeGenericHandoffReply
// ---------------------------------------------------------------------------

#[test]
fn handoff_markers() {
    assert!(looks_like_generic_handoff_reply(
        "what do you want",
        "Tell me your question or your task."
    ));
    assert!(looks_like_generic_handoff_reply(
        "how can I help?",
        "I remain at your disposal."
    ));
}

#[test]
fn assist_regex() {
    assert!(looks_like_generic_handoff_reply(
        "something",
        "How may I assist you today?"
    ));
}

#[test]
fn greeting_user_not_handoff() {
    assert!(!looks_like_generic_handoff_reply(
        "hello",
        "Tell me your question or your task."
    ));
}

#[test]
fn handoff_normal_reply() {
    assert!(!looks_like_generic_handoff_reply(
        "check the weather",
        "Sunny today."
    ));
}

#[test]
fn blank_reply() {
    assert!(!looks_like_generic_handoff_reply("hi", ""));
}

// ---------------------------------------------------------------------------
// TestIsDuplicateEvent
// ---------------------------------------------------------------------------

#[test]
fn first_seen_false_then_true() {
    let mut recent = RecentEvents::new(5);
    assert!(!recent.is_duplicate(&json!("evt-1")));
    assert!(recent.is_duplicate(&json!("evt-1")));
}

#[test]
fn new_ids_appended() {
    let mut recent = RecentEvents::new(5);
    let _ = recent.is_duplicate(&json!("evt-1"));
    assert!(!recent.is_duplicate(&json!("evt-2")));
    assert_eq!(
        recent.recent().cloned().collect::<Vec<Value>>(),
        vec![json!("evt-1"), json!("evt-2")]
    );
}

#[test]
fn maxlen_eviction() {
    let mut recent = RecentEvents::new(1);
    assert!(!recent.is_duplicate(&json!("a")));
    assert!(!recent.is_duplicate(&json!("b")));
    // "a" was evicted by "b", so it is seen fresh again.
    assert!(!recent.is_duplicate(&json!("a")));
    assert_eq!(
        recent.recent().cloned().collect::<Vec<Value>>(),
        vec![json!("a")]
    );
}

#[test]
fn none_id() {
    // An event with no id is remembered and matched like any other.
    let mut recent = RecentEvents::new(5);
    assert!(!recent.is_duplicate(&Value::Null));
    assert!(recent.is_duplicate(&Value::Null));
}

// ---------------------------------------------------------------------------
// TestStripToolCallEnvelope
// ---------------------------------------------------------------------------

#[test]
fn removes_a_closed_envelope() {
    let text = "Let me look. {tool_calls}{\"tool\": \"search_web\", \"args\": {}}{/tool_calls}";

    // The two spaces either side of the envelope stay: the pattern removes the
    // markup and nothing else.
    assert_eq!(strip_tool_call_envelope(text), "Let me look.  ");
}

#[test]
fn removes_a_lone_tag() {
    // An envelope opened and never closed leaves its tag behind. The tag is
    // markup wherever it appears; what it was carrying is left alone, because
    // guessing where an unclosed envelope ends would sometimes delete a real
    // answer.
    assert_eq!(
        strip_tool_call_envelope("Let me look. {tool_calls}"),
        "Let me look.  "
    );
}

#[test]
fn tolerates_spacing_and_case_in_the_tag() {
    let text = "Hi. { TOOL_CALLS }body{ / tool_calls }";

    assert!(!strip_tool_call_envelope(text)
        .to_lowercase()
        .contains("tool_calls"));
}

#[test]
fn removes_every_envelope() {
    let text = "{tool_calls}a{/tool_calls} middle {tool_calls}b{/tool_calls}";

    assert_eq!(strip_tool_call_envelope(text).trim(), "middle");
}

#[test]
fn leaves_tone_tags_alone() {
    // The whole safety argument: a tone tag is load-bearing markup for the
    // voice, and none of the tone names is "tool_calls".
    for text in [
        "{somber}I'm sorry{/somber}",
        "{whisper}don't tell anyone{/whisper}",
        "{cheerful}that's wonderful{/cheerful}",
        "No markup here at all.",
    ] {
        assert_eq!(strip_tool_call_envelope(text), text, "{text}");
    }
}

#[test]
fn empty_input() {
    assert_eq!(strip_tool_call_envelope(""), "");
    assert_eq!(strip_tool_call_envelope(""), "");
}

// ---------------------------------------------------------------------------
// TestToolCallsInEnvelope
// ---------------------------------------------------------------------------

#[test]
fn single_object() {
    let text =
        "Hm. {tool_calls}{\"tool\": \"search_web\", \"args\": {\"query\": \"x\"}}{/tool_calls}";

    assert_eq!(
        tool_calls_in_envelope(text),
        vec![json!({ "tool": "search_web", "args": { "query": "x" } })]
    );
}

#[test]
fn array_form() {
    // The schema itself shows tool_calls as an array, so the model may put an
    // array inside the tag.
    let text =
        "{tool_calls}[{\"tool\": \"a\", \"args\": {}}, {\"tool\": \"b\", \"args\": {\"n\": 1}}]{/tool_calls}";

    assert_eq!(
        tool_calls_in_envelope(text),
        vec![
            json!({ "tool": "a", "args": {} }),
            json!({ "tool": "b", "args": { "n": 1 } })
        ]
    );
}

#[test]
fn every_envelope_is_read() {
    let text = "{tool_calls}{\"tool\": \"a\", \"args\": {}}{/tool_calls}\
                {tool_calls}{\"tool\": \"b\", \"args\": {}}{/tool_calls}";

    assert_eq!(
        tool_calls_in_envelope(text)
            .iter()
            .map(|call| call["tool"].clone())
            .collect::<Vec<Value>>(),
        vec![json!("a"), json!("b")]
    );
}

#[test]
fn args_that_are_not_an_object_become_empty() {
    let text = "{tool_calls}{\"tool\": \"search_web\", \"args\": \"latest news\"}{/tool_calls}";

    assert_eq!(
        tool_calls_in_envelope(text),
        vec![json!({ "tool": "search_web", "args": {} })]
    );
}

#[test]
fn a_call_with_no_name_is_dropped() {
    for body in [
        "{\"args\": {}}",
        "{\"tool\": \"\"}",
        "{\"tool\": \"   \"}",
        "{\"tool\": null}",
    ] {
        let text = format!("{{tool_calls}}{body}{{/tool_calls}}");
        assert_eq!(tool_calls_in_envelope(&text), Vec::<Value>::new(), "{body}");
    }
}

#[test]
fn a_non_dict_entry_is_dropped() {
    // The tool runner indexes into every call it is given, so a bare string in
    // the array has to be dropped here rather than raised on there.
    let text = "{tool_calls}[\"search_web\", {\"tool\": \"a\", \"args\": {}}]{/tool_calls}";

    assert_eq!(
        tool_calls_in_envelope(text),
        vec![json!({ "tool": "a", "args": {} })]
    );
}

#[test]
fn a_body_that_is_not_json_yields_nothing() {
    for body in ["", "   ", "not json", "{\"tool\": \"unterminated"] {
        let text = format!("{{tool_calls}}{body}{{/tool_calls}}");
        assert_eq!(tool_calls_in_envelope(&text), Vec::<Value>::new(), "{body}");
    }
}

#[test]
fn a_json_scalar_yields_nothing() {
    // Parses as JSON, but names no call.
    for body in ["5", "\"search_web\"", "null", "true"] {
        let text = format!("{{tool_calls}}{body}{{/tool_calls}}");
        assert_eq!(tool_calls_in_envelope(&text), Vec::<Value>::new(), "{body}");
    }
}

#[test]
fn no_envelope() {
    assert_eq!(
        tool_calls_in_envelope("Just a plain reply."),
        Vec::<Value>::new()
    );
    assert_eq!(tool_calls_in_envelope(""), Vec::<Value>::new());
    assert_eq!(tool_calls_in_envelope(""), Vec::<Value>::new());
}

#[test]
fn a_lone_tag_carries_nothing() {
    assert_eq!(
        tool_calls_in_envelope("Let me look. {tool_calls}"),
        Vec::<Value>::new()
    );
}

// ---------------------------------------------------------------------------
// quality.json — every case the capture script recorded.
//
// The tests above are the reference app's own assertions, transcribed. These
// walk the golden file, which holds the inputs the Python test file never
// thought of (a reply of exactly the teaser limit plus a colon, a handoff whose
// user text is `None`, an envelope tag with a newline in it) alongside the ones
// it did.
// ---------------------------------------------------------------------------

#[test]
fn fixture_constants() {
    let doc = fixture("quality.json");
    let constants = &doc["constants"];
    let count = |key: &str| -> usize {
        usize::try_from(
            constants[key]
                .as_u64()
                .unwrap_or_else(|| panic!("constant {key} is not a count: {constants}")),
        )
        .unwrap_or_default()
    };
    assert_eq!(REPLY_SEGMENT_MAX, count("REPLY_SEGMENT_MAX"));
    assert_eq!(TEASER_MAX_WORDS, count("TEASER_MAX_WORDS"));
}

#[test]
fn fixture_looks_like_action_teaser_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_like_action_teaser_reply") {
        assert_eq!(
            looks_like_action_teaser_reply(input_text(case, "input")),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_like_mctrader_action_request() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_like_mctrader_action_request") {
        assert_eq!(
            looks_like_mctrader_action_request(input_text(case, "input")),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_incomplete() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_incomplete") {
        let details = match &case["response_details"] {
            Value::Null => None,
            other => Some(other),
        };
        assert_eq!(
            looks_incomplete(input_text(case, "reply_text"), details),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_latest_user_message_text() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "latest_user_message_text") {
        assert_eq!(
            latest_user_message_text(array(case, "messages")),
            expected_text(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_like_task_stub_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_like_task_stub_reply") {
        assert_eq!(
            looks_like_task_stub_reply(
                input_text(case, "user_text"),
                input_text(case, "reply_text")
            ),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_merge_completed_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "merge_completed_reply") {
        assert_eq!(
            merge_completed_reply(input_text(case, "partial"), input_text(case, "completion")),
            expected_text(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_extract_reply_segments() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "extract_reply_segments") {
        assert_eq!(
            extract_reply_segments(value(case, "value")),
            text_list(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_strip_tool_call_envelope() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "strip_tool_call_envelope") {
        assert_eq!(
            strip_tool_call_envelope(input_text(case, "input")),
            expected_text(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_tool_calls_in_envelope() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "tool_calls_in_envelope") {
        assert_eq!(
            json!(tool_calls_in_envelope(input_text(case, "input"))),
            *value(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_unusable_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_unusable_reply") {
        assert_eq!(
            looks_unusable_reply(input_text(case, "input")),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_out_of_character_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_out_of_character_reply") {
        assert_eq!(
            looks_out_of_character_reply(input_text(case, "input")),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_like_recovery_meta_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_like_recovery_meta_reply") {
        assert_eq!(
            looks_like_recovery_meta_reply(input_text(case, "input")),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_looks_like_generic_handoff_reply() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "looks_like_generic_handoff_reply") {
        assert_eq!(
            looks_like_generic_handoff_reply(
                input_text(case, "user_text"),
                input_text(case, "reply_text")
            ),
            expected_bool(case, "expected"),
            "{case}"
        );
    }
}

#[test]
fn fixture_events() {
    let doc = fixture("quality.json");
    for case in cases(&doc, "events") {
        let limit = usize::try_from(case["maxlen"].as_u64().unwrap_or_default())
            .unwrap_or_else(|_| panic!("case maxlen is not a count: {case}"));
        let mut recent = RecentEvents::new(limit);
        for (event_id, expected) in array(case, "ids").iter().zip(bools(case, "expected")) {
            assert_eq!(recent.is_duplicate(event_id), expected, "{case}");
        }
        assert_eq!(
            recent.recent().collect::<Vec<&Value>>(),
            array(case, "final").iter().collect::<Vec<&Value>>(),
            "{case}"
        );
    }
}
