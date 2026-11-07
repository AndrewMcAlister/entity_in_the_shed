//! A response body written on one line, for a log line or an error message.
//!
//! Ported from `utils/text.py:78` (`_safe_body_snippet`), which is what every
//! "the engine said something odd" message in the source uses: `agent.py:1220`,
//! `3975`, `4061`, `7053`, `7093`, `7111`, `7126` and `utils/json_util.py:53`, `62`.

/// `_safe_body_snippet`'s `max_len` default (`utils/text.py:78`).
///
/// No caller in the source passes another length, so this is the only length
/// the helper has.
pub const MAX_SNIPPET_CHARS: usize = 300;

/// Flatten a body into a single line and cut it off at [`MAX_SNIPPET_CHARS`]
/// characters.
///
/// The length is counted in characters, not bytes, because `len(value)` in
/// Python counts characters and `value[:max_len]` slices characters: a body of
/// 400 accented letters is truncated at 400 letters, not at 400 UTF-8 bytes.
///
/// Only `\r` and `\n` become spaces, each on its own, so a `\r\n` pair becomes
/// two of them. A tab survives, because the source only replaces those two
/// characters, and a tab inside a JSON body is the body's own formatting rather
/// than a line break the log would have to wrap.
pub fn safe_body_snippet(text: &str) -> String {
    let flattened = text.replace(['\r', '\n'], " ");
    let value = flattened.trim();

    // `value.char_indices().nth(MAX)` is the character just past the budget:
    // `Some` means the body is longer than it, and its offset is exactly where
    // the slice must end.
    match value.char_indices().nth(MAX_SNIPPET_CHARS) {
        Some((at, _)) => format!("{}...", &value[..at]),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_body_is_flattened_but_not_truncated() {
        assert_eq!(safe_body_snippet("line1\nline2"), "line1 line2");
    }

    #[test]
    fn a_long_body_is_cut_at_the_limit_and_marked_with_an_ellipsis() {
        let long = safe_body_snippet(&"x".repeat(400));
        assert_eq!(
            long.chars().count(),
            MAX_SNIPPET_CHARS + 3,
            "300 characters plus \"...\""
        );
        assert!(long.ends_with("..."));
    }

    #[test]
    fn carriage_returns_are_flattened_and_the_ends_are_trimmed() {
        // Each of the two characters becomes its own space, so a `\r\n` pair is
        // two spaces: the source replaces one and then the other, and only the
        // ends are trimmed.
        assert_eq!(
            safe_body_snippet("  <html>\r\n<body>\r\n  "),
            "<html>  <body>"
        );
    }

    #[test]
    fn a_body_exactly_at_the_limit_is_left_alone() {
        // `len(value) <= max_len` is the source's comparison, so the limit
        // itself is not a cut.
        let exact = safe_body_snippet(&"y".repeat(MAX_SNIPPET_CHARS));
        assert_eq!(exact.chars().count(), MAX_SNIPPET_CHARS);
        assert!(!exact.ends_with("..."));
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let body = "é".repeat(400);
        let snippet = safe_body_snippet(&body);
        assert_eq!(snippet.chars().count(), MAX_SNIPPET_CHARS + 3);
        assert!(snippet.starts_with(&"é".repeat(MAX_SNIPPET_CHARS)));
    }

    #[test]
    fn an_empty_body_snippets_as_an_empty_string() {
        assert_eq!(safe_body_snippet(""), "");
        assert_eq!(safe_body_snippet("   \n  \r  "), "");
    }
}
