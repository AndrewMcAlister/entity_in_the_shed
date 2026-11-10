//! `tests/test_app_stylesheet.py` — the stylesheet served at `/app.css` must
//! survive CSS parsing, not merely reading.
//!
//! The Python docstring is the whole reason this file exists, so it is kept
//! here rather than paraphrased: a declaration block with no selector in front
//! of it is not discarded on its own. The parser keeps consuming as it looks
//! for the `{` that opens the block, so the orphan declarations become the
//! prelude of the *next* rule and that rule is thrown away with them. Deleting
//! one line — `body {` — therefore also deleted `.shell`, and the page
//! rendered edge to edge with the Send and Send Password buttons drawn over
//! the fields beside them.
//!
//! Nothing noticed, because the old check was `b".shell" in css`: the text was
//! still in the file, just no longer a rule. So these tests parse, and read
//! the stylesheet from disk because that is what the `/app.css` route serves
//! (pinned by `frontend_cache_headers`).

//! The test binary links these crates through the library but names none of
//! them itself; the workspace's `unused_crate_dependencies` lint judges the
//! binary on its own uses.
use axum as _;
use http_body_util as _;
use serde as _;
use serde_json as _;
use shed_characters as _;
use shed_config as _;
use shed_llm as _;
use shed_models as _;
use shed_text as _;
use shed_tts as _;
use tokio as _;
use tower as _;
use tracing as _;
use uuid as _;

use std::path::{Path, PathBuf};

/// The repository root, which is the clone's `BASE_DIR`: the file layout the
/// routes read.
fn base_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `strip_comments` — every comment becomes a single space, so it cannot join
/// a prelude.
///
/// The Python original is `re.sub(r"/\*.*?\*/", " ", …, flags=re.DOTALL)`. The
/// non-greedy scan is four lines of hand-rolled matching here, and the reason
/// it is hand-rolled is that a regex would need a third-party flavour to mean
/// "non-greedy across newlines" (`fancy-regex`), which is a dependency spent
/// on nothing.
fn strip_comments(stylesheet: &str) -> String {
    let mut stripped = String::with_capacity(stylesheet.len());
    let mut rest = stylesheet;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        stripped.push(' ');
        // `.*?` stops at the first `*/`; an unterminated comment runs to the
        // end of the file, exactly as the regex does.
        rest = match rest[start + 2..].find("*/") {
            Some(offset) => &rest[start + 2 + offset + 2..],
            None => "",
        };
    }
    stripped.push_str(rest);
    stripped
}

/// `parse_top_level_preludes` — the text in front of every top-level `{`, in
/// source order.
///
/// Nested blocks (the inside of an `@media`) are not returned; only the rules
/// and at-rules of the stylesheet itself. Strings are not treated as opaque,
/// which is safe here because no string in this stylesheet contains a brace.
///
/// The `Err` is the Python `ValueError` for an unbalanced brace: a stylesheet
/// that cannot be scanned is not a stylesheet with good preludes, and a caller
/// that ignored this would report "every block has a selector" for a file
/// truncated mid-rule.
fn parse_top_level_preludes(stylesheet: &str) -> Result<Vec<String>, String> {
    let stylesheet = strip_comments(stylesheet);
    let mut preludes = Vec::new();
    let mut depth = 0usize;
    let mut prelude_start = 0usize;

    for (index, character) in stylesheet.char_indices() {
        match character {
            '{' => {
                if depth == 0 {
                    preludes.push(stylesheet[prelude_start..index].to_owned());
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    prelude_start = index + character.len_utf8();
                }
            }
            // A `}` with nothing open is left in the text, so it shows up in
            // the offending prelude rather than ending the scan before it is
            // reported.
            _ => {}
        }
    }

    if depth != 0 {
        return Err("unbalanced opening brace".to_owned());
    }
    Ok(preludes)
}

/// `setUp`: the shipped stylesheet, with preludes trimmed as the Python test
/// trims them.
fn shipped_preludes() -> Vec<String> {
    let path = shed_http::page::PagePaths::new(&base_dir()).stylesheet;
    let stylesheet = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    parse_top_level_preludes(&stylesheet)
        .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()))
        .iter()
        .map(|prelude| prelude.trim().to_owned())
        .collect()
}

#[test]
fn no_declaration_block_is_missing_its_selector() {
    // The signature of the bug: a block whose prelude holds declarations that
    // belong to a rule which lost its selector line.
    for prelude in shipped_preludes() {
        assert!(
            !prelude.contains(';'),
            "declarations in the prelude of a block: {prelude:?}"
        );
        assert!(
            !prelude.contains('}'),
            "stray brace before a block: {prelude:?}"
        );
        assert!(
            !prelude.is_empty(),
            "a top-level block has no selector in front of it"
        );
    }
}

#[test]
fn the_page_frame_rules_are_present_as_rules() {
    // Deleting any of these outright leaves a parseable stylesheet, so the
    // structural test above would not notice.
    let preludes = shipped_preludes();
    assert!(
        preludes.iter().any(|prelude| prelude == "body"),
        "body carries the page padding and background"
    );
    assert!(
        preludes.iter().any(|prelude| prelude == ".shell"),
        ".shell caps and centres the card"
    );
    assert!(
        preludes.iter().any(|prelude| prelude == "*"),
        "the global reset keeps padding inside widths"
    );
}

#[test]
fn the_scanner_itself_matches_the_regex_it_replaces() {
    // The one line of the port that could be quietly wrong: a comment that
    // spans lines, one that never closes, and the braces it hides.
    assert_eq!(strip_comments("a /* one\n two */ b"), "a   b");
    assert_eq!(strip_comments("a /* unterminated {"), "a  ");
    // A brace inside a comment is not a brace, and a comment inside a prelude
    // is a space, not a join: `/* { */ body` becomes two spaces then `body`,
    // because the comment collapses to one space and the space that followed
    // it is still there. Verified against `strip_comments`'s regex run under
    // the reference's own interpreter.
    let preludes = parse_top_level_preludes(
        "/* { */ body { margin: 0 } @media (width > 10px) { .a { b: c } }",
    )
    .unwrap_or_else(|error| panic!("the sample should balance: {error}"));
    assert_eq!(preludes, ["  body ", " @media (width > 10px) "]);
}
