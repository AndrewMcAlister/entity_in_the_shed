//! The sentence splitter: where a streamed reply is cut into speakable clips.
//!
//! A port of `tests/test_sentence_splitter.py`: all 21 of its tests, names kept
//! verbatim (they are unique in the Python file, so nothing needed a prefix),
//! plus every case of `tests/fixtures/splitter.json` — the reference app's own
//! output, captured by `tools/capture_reply_fixtures.py` for the 23 texts the
//! Python test file never fed the splitter, including the two markup cases
//! (`underscores`, `mixed_markup`) that exist only in the fixture.
//!
//! Three things the Python never has to say and this file does:
//!
//! * `split(chunks)` takes a list, and three of the reference tests hand it a
//!   *string*. Python iterates it, so those calls feed one character at a time;
//!   the port says [`split_streamed`] with size 1 rather than pretend a whole
//!   reply arrived in one piece.
//! * Python's `len(sentence)` counts characters and its `str.split()` splits on
//!   them, so the port counts with [`str::chars`] rather than bytes. The texts
//!   here are ASCII, where the two agree; the difference would only show on a
//!   reply the fixture does not contain.
//! * The fixture's seeded chunk sequences are recorded in it
//!   (`seeded_chunks`), because they come from Python's `random.Random` and a
//!   reader of the JSON could not otherwise rebuild the cut that produced the
//!   answer.

mod common;

use serde_json::{Map, Value};
use shed_chat::{SentenceSplitter, COMMON_NON_TERMINAL_ABBREVIATIONS};

use common::{cases, field, fixture, value};

// ---------------------------------------------------------------------------
// The two drivers of the reference file, transcribed.
// ---------------------------------------------------------------------------

/// Feed a reply through the splitter the way streaming would.
fn split(chunks: &[Option<&str>]) -> Vec<String> {
    let mut splitter = SentenceSplitter::new();
    let mut sentences = Vec::new();
    for chunk in chunks {
        sentences.extend(splitter.feed(*chunk));
    }
    sentences.extend(splitter.flush());
    sentences
}

/// `text` in consecutive pieces of `size` characters, as Python slices it.
///
/// No caller asks for pieces of zero: Python's `range` raises on that step,
/// where a Rust loop would spin, so the clamp is a guard and never a rule.
fn pieces(text: &str, size: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = start;
        for _ in 0..size.max(1) {
            let Some(ch) = text[end..].chars().next() else {
                break;
            };
            end += ch.len_utf8();
        }
        out.push(&text[start..end]);
        start = end;
    }
    out
}

fn split_streamed(text: &str, chunk_size: usize) -> Vec<String> {
    let chunks: Vec<Option<&str>> = pieces(text, chunk_size).into_iter().map(Some).collect();
    split(&chunks)
}

/// A fixture value that is a list of sentences.
fn strings(list: &Value) -> Vec<&str> {
    let items = list
        .as_array()
        .unwrap_or_else(|| panic!("expected a list of sentences, found {list}"));
    items
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("expected a list of sentences, found {list}"))
        })
        .collect()
}

/// A fixture value that is a list of the lists one `feed` returns.
fn groups(list: &Value) -> Vec<Vec<&str>> {
    let items = list
        .as_array()
        .unwrap_or_else(|| panic!("expected a list of lists, found {list}"));
    items.iter().map(strings).collect()
}

/// A fixture value that is an object keyed by chunk size or by seed.
fn keyed(list: &Value) -> &Map<String, Value> {
    list.as_object()
        .unwrap_or_else(|| panic!("expected a keyed object, found {list}"))
}

fn count(value: &Value, key: &str) -> usize {
    usize::try_from(
        value[key]
            .as_u64()
            .unwrap_or_else(|| panic!("count {key} is not a number: {value}")),
    )
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// tests/test_sentence_splitter.py
// ---------------------------------------------------------------------------

#[test]
fn splits_on_sentence_terminators() {
    assert_eq!(
        split_streamed(
            "Let me check that for you. It is forty-two! Are you sure?",
            1
        ),
        [
            "Let me check that for you.",
            "It is forty-two!",
            "Are you sure?"
        ]
    );
}

#[test]
fn chunking_does_not_change_the_result() {
    let text = "First sentence here. Second sentence there. Third one to finish.";
    let expected = split(&[Some(text)]);
    for size in [1, 2, 3, 7, 50, 1000] {
        assert_eq!(split_streamed(text, size), expected, "chunk size {size}");
    }
}

#[test]
fn abbreviations_do_not_end_a_sentence() {
    assert_eq!(
        split_streamed("I spoke to Mr. Smith about it. He agreed with me.", 1),
        ["I spoke to Mr. Smith about it.", "He agreed with me."]
    );
}

#[test]
fn dotted_acronyms_do_not_end_a_sentence() {
    assert_eq!(
        split_streamed("The U.S. market is up today. That is good news.", 1),
        ["The U.S. market is up today.", "That is good news."]
    );
}

#[test]
fn decimals_do_not_end_a_sentence() {
    assert_eq!(
        split_streamed("The value is 3.5 percent higher. That matters.", 1),
        ["The value is 3.5 percent higher.", "That matters."]
    );
}

#[test]
fn version_like_numbers_still_end_a_sentence() {
    assert_eq!(
        split_streamed("We are on version 2. It works well enough.", 1),
        ["We are on version 2.", "It works well enough."]
    );
}

#[test]
fn ellipsis_ends_a_sentence() {
    assert_eq!(
        split_streamed("Well now then... that is a long story indeed. Sit down.", 1),
        [
            "Well now then...",
            "that is a long story indeed.",
            "Sit down."
        ]
    );
}

#[test]
fn short_fragments_merge_with_the_next_sentence() {
    // "Ha." and "Yes." are not worth a synthesis request each.
    assert_eq!(
        split_streamed("Ha. Yes. That is very funny indeed.", 1),
        ["Ha. Yes. That is very funny indeed."]
    );
}

#[test]
fn no_split_inside_a_tone_tag() {
    assert_eq!(
        split_streamed(
            "She said {somber}I am sorry{/somber} to hear it. Then she left.",
            1
        ),
        [
            "She said {somber}I am sorry{/somber} to hear it.",
            "Then she left."
        ]
    );
}

#[test]
fn no_split_inside_a_scene_bracket() {
    assert_eq!(
        split_streamed(
            "He paused for a moment. [the kettle whistles. nobody moves] Then he spoke again.",
            1
        ),
        [
            "He paused for a moment.",
            "[the kettle whistles. nobody moves] Then he spoke again."
        ]
    );
}

#[test]
fn no_split_inside_an_aside() {
    assert_eq!(
        split_streamed(
            "It rained all week. (which is normal here. everyone knows it) We stayed in.",
            1
        ),
        [
            "It rained all week.",
            "(which is normal here. everyone knows it) We stayed in."
        ]
    );
}

#[test]
fn an_unclosed_opener_holds_the_boundary_but_not_the_reply() {
    // Half an aside is not markup, so no full stop inside it can be trusted
    // -- and the client's own splitter refuses the same boundary
    // (splitIntoSentences in static/app-helpers.js), so both sides cut a
    // reply in the same places. End of text still releases the words: the
    // opener holds the boundary, never the reply.
    let mut splitter = SentenceSplitter::new();

    assert_eq!(
        splitter.feed(Some(
            "It rained all week. (which is normal here. everyone knows it"
        )),
        ["It rained all week."]
    );
    assert_eq!(
        splitter.flush(),
        ["(which is normal here. everyone knows it"]
    );
}

#[test]
fn no_split_inside_emphasis() {
    assert_eq!(
        split_streamed("He was *very. truly. annoyed* about it. We noticed.", 1),
        ["He was *very. truly. annoyed* about it.", "We noticed."]
    );
}

#[test]
fn trailing_text_without_a_terminator_arrives_at_flush() {
    let mut splitter = SentenceSplitter::new();
    assert_eq!(
        splitter.feed(Some("A complete sentence here. And then some text")),
        ["A complete sentence here."]
    );
    assert_eq!(splitter.flush(), ["And then some text"]);
    assert_eq!(splitter.flush(), Vec::<String>::new());
}

#[test]
fn flush_of_a_dangling_terminator_emits_it() {
    // The Python passes `split` a string, which it iterates: one character a
    // time, so the terminator reaches the splitter the way a stream's last
    // token would.
    assert_eq!(
        split_streamed("Just one sentence now.", 1),
        ["Just one sentence now."]
    );
}

#[test]
fn flush_of_whitespace_only_emits_nothing() {
    assert_eq!(split_streamed("   \n  ", 1), Vec::<String>::new());
}

#[test]
fn long_unbroken_text_is_not_cut_on_a_comma() {
    // A comma is no longer a forced cut: a clip boundary there is heard as
    // an over-long pause after the comma. The sentence falls back to the
    // word-boundary cut instead.
    let text = format!(
        "This sentence runs on and on, {}and finally stops",
        "and on ".repeat(60)
    );
    let sentences = split_streamed(&text, 40);

    assert!(sentences.len() > 1);
    for sentence in &sentences {
        assert!(sentence.chars().count() <= SentenceSplitter::MAX_SENTENCE_CHARS);
        assert!(!sentence.ends_with(','));
    }
}

#[test]
fn long_text_is_still_cut_on_a_semicolon() {
    // The softer breaks stay breaks; only the comma left the list.
    let text = format!(
        "First clause here; {}and finally stops",
        "and on ".repeat(60)
    );
    let sentences = split_streamed(&text, 40);

    assert!(sentences.len() > 1);
    assert!(sentences[0].ends_with(';'));
}

#[test]
fn long_text_without_any_punctuation_is_still_bounded() {
    let text = "word ".repeat(200);
    let sentences = split_streamed(&text, 30);

    assert!(sentences.len() > 1);
    for sentence in &sentences {
        assert!(sentence.chars().count() <= SentenceSplitter::MAX_SENTENCE_CHARS);
    }
}

#[test]
fn every_sentence_is_non_empty_and_stripped() {
    let text = "  One thing here.   Two things there!  Three.  ";
    for sentence in split_streamed(text, 5) {
        assert!(!sentence.is_empty());
        assert_eq!(sentence, sentence.trim());
    }
}

#[test]
fn reassembled_output_keeps_all_the_words() {
    let text = "Right, let me check that for you. The U.S. market is up 3.5 percent, which is \
                good news... Anyway, that is the lot.";
    let sentences = split_streamed(text, 3);

    assert_eq!(
        sentences.join(" ").split_whitespace().collect::<Vec<_>>(),
        text.split_whitespace().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// splitter.json — every case the capture script recorded.
//
// The tests above are the reference app's own assertions, transcribed. These
// walk the golden file: the same text fed as one chunk, then in pieces of ten
// different sizes, then at two seeded cuts, then three characters at a time so
// that the *grouping* of the output — which feed released which sentence, and
// what `flush` was left to say — is pinned as well as the sentences themselves.
// ---------------------------------------------------------------------------

#[test]
fn fixture_constants() {
    let doc = fixture("splitter.json");
    assert_eq!(
        SentenceSplitter::MIN_SENTENCE_CHARS,
        count(&doc, "min_sentence_chars")
    );
    assert_eq!(
        SentenceSplitter::MAX_SENTENCE_CHARS,
        count(&doc, "max_sentence_chars")
    );

    // The fixture sorts the list; the constant is written in that order, and
    // the sort here is what says so rather than an assumption about it.
    let mut mine = COMMON_NON_TERMINAL_ABBREVIATIONS.to_vec();
    mine.sort_unstable();
    assert_eq!(mine, strings(&doc["common_non_terminal_abbreviations"]));
}

#[test]
fn fixture_splitter_whole_and_by_chunk_size() {
    let doc = fixture("splitter.json");
    for case in cases(&doc, "cases") {
        let label = field(case, "label");
        let text = field(case, "text");
        assert_eq!(
            split(&[Some(text)]),
            strings(value(case, "whole")),
            "{label} whole"
        );

        for (size, expected) in keyed(value(case, "by_chunk_size")) {
            let size = usize::try_from(size.parse::<u64>().unwrap_or_else(|err| {
                panic!("chunk size {size} of {label} is not a number: {err}")
            }))
            .unwrap_or_default();
            assert_eq!(
                split_streamed(text, size),
                strings(expected),
                "{label} chunk size {size}"
            );
        }
    }
}

#[test]
fn fixture_splitter_seeded_chunks() {
    let doc = fixture("splitter.json");
    for case in cases(&doc, "cases") {
        let label = field(case, "label");
        let seeded = value(case, "seeded");
        for (seed, chunks) in keyed(value(case, "seeded_chunks")) {
            let fed: Vec<Option<&str>> = strings(chunks).into_iter().map(Some).collect();
            assert_eq!(split(&fed), strings(&seeded[seed]), "{label} seeded {seed}");
        }
    }
}

#[test]
fn fixture_splitter_feed_groups_and_flush() {
    let doc = fixture("splitter.json");
    for case in cases(&doc, "cases") {
        let label = field(case, "label");
        let mut splitter = SentenceSplitter::new();
        let mut fed: Vec<Vec<String>> = Vec::new();
        for chunk in pieces(field(case, "text"), 3) {
            let got = splitter.feed(Some(chunk));
            if !got.is_empty() {
                fed.push(got);
            }
        }
        assert_eq!(fed, groups(value(case, "feed_groups")), "{label} feeds");
        assert_eq!(
            splitter.flush(),
            strings(value(case, "flush")),
            "{label} flush"
        );
        assert_eq!(
            splitter.flush(),
            strings(value(case, "flush_again")),
            "{label} flush again"
        );
    }
}
