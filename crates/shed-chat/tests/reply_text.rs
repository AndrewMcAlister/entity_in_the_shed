//! `agent.py`'s two reply-text passes: `normalize_reply_text` and
//! `markdown_to_ssml`.
//!
//! There is no reference test for either of them on its own — the Python suite
//! meets `normalize_reply_text` only inside `test_reply_recovery.py` and
//! `test_text_utils.py`, where it is one call inside an assertion about
//! recovery or about a *different* function, and it meets `markdown_to_ssml`
//! nowhere at all. So the rows here are the oracle: every input and every
//! expected output in `reply_text.json` was captured from the reference app,
//! which is the only statement of what these two functions do that does not
//! come from the Rust.
//!
//! The rows are worth naming, because they are not the obvious ones:
//!
//! * `normalize_reply_text` is asserted on the raw JSON value the Python is
//!   handed, `null` included, rather than on a string — the falsy test in
//!   `str(value or "")` is part of the behaviour.
//! * The double space after a sentence stop is asserted next to the single
//!   space after an abbreviation, and next to `No.Say it.`, which gains one
//!   boundary and not two because `re.sub` resumes after the match.
//! * `markdown_to_ssml` is asserted on `**a*b*c**`, where the order of the
//!   three passes and their laziness are the whole result.

mod common;

use common::{cases, expected_text, fixture, input_text};
use shed_chat::{markdown_to_ssml, normalize_reply_text};

#[test]
fn fixture_normalize_reply_text() {
    let doc = fixture("reply_text.json");
    for case in cases(&doc, "normalize_reply_text") {
        // The whole captured value goes in, `null` included: the Python reads
        // `str(value or "")`, so the assertion is about the value and not
        // about a string someone chose to represent it.
        let got = normalize_reply_text(common::value(case, "input"));
        assert_eq!(
            got,
            expected_text(case, "expected"),
            "normalize_reply_text({})",
            common::value(case, "input")
        );
    }
}

#[test]
fn fixture_markdown_to_ssml() {
    let doc = fixture("reply_text.json");
    for case in cases(&doc, "markdown_to_ssml") {
        assert_eq!(
            markdown_to_ssml(input_text(case, "input")),
            expected_text(case, "expected"),
            "markdown_to_ssml({:?})",
            input_text(case, "input")
        );
    }
}
