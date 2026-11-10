//! Python's own `str` methods, spelled the way the interpreter spells them:
//! `str.title()` and `str.splitlines()`.
//!
//! These two are not `utils/` helpers — they are language behaviour the source
//! leans on inline, and a Rust `to_case`/`lines()` would answer differently:
//! `title()` lower-cases every letter after the first of a word (so `"it's"`
//! becomes `"It'S"`), and `splitlines()` breaks on eight line boundaries beyond
//! `\n`, `\r\n` and `\r`.
//!
//! They live here because two crates that may not depend on each other quote
//! them: `shed-characters` (`character_label_marker`'s `.title()`,
//! `character_text_lines`' `.splitlines()`) and `shed-prompts`
//! (`utils/prompt.py:52`'s titled `Sex`/`SubjectPronoun` variables, and the
//! `.splitlines()` at `utils/prompt.py:26` that reads a template's
//! frontmatter). The `str()`/`repr()` idioms over JSON values went to
//! [`crate::value_text`] for the same reason.

/// Python's `str.title()`: the first alphabetic character of every word
/// upper-cases and every later one lower-cases, where a word begins after any
/// non-alphabetic character.
///
/// The apostrophe is a word boundary, so Python spells `"it's"` as `"It'S"` —
/// kept deliberately: `character_label_marker` titles a key suffix like
/// `"gwyn's-hall"`, and a prettier Rust title would be a second answer.
#[must_use]
pub fn py_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_word = false;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            if in_word {
                out.extend(ch.to_lowercase());
            } else {
                out.extend(ch.to_uppercase());
                in_word = true;
            }
        } else {
            in_word = false;
            out.push(ch);
        }
    }
    out
}

/// Python's `str.splitlines()`: split on every line boundary the language
/// knows, with no trailing empty element for a final line break.
///
/// Beyond `\n`, `\r\n` and `\r`, Python also breaks on the vertical tab, the
/// form feed, the file/group/record separators, NEL and the two Unicode line
/// separators; an authored backstory string can hold any of them, so the set
/// is carried whole rather than trimmed to the common three.
#[must_use]
pub fn py_splitlines(text: &str) -> Vec<&str> {
    const BREAKS: [char; 9] = [
        '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}', '\n',
    ];
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        let break_len = if ch == '\r' {
            // `\r\n` is one boundary, not two.
            if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                2
            } else {
                1
            }
        } else if BREAKS.contains(&ch) {
            1
        } else {
            continue;
        };
        lines.push(&text[start..index]);
        start = index + ch.len_utf8() + usize::from(break_len == 2);
        if break_len == 2 {
            chars.next();
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_follows_pythons_word_boundaries() {
        assert_eq!(py_title("cans"), "Cans");
        assert_eq!(py_title("iu-lATN"), "Iu-Latn");
        // The apostrophe starts a new word, exactly like Python.
        assert_eq!(py_title("gwyn's-hall"), "Gwyn'S-Hall");
        assert_eq!(py_title(""), "");
        assert_eq!(py_title("2nd"), "2Nd");
    }

    #[test]
    fn splitlines_knows_every_python_break() {
        assert_eq!(py_splitlines("a\r\nb\rc\nd"), ["a", "b", "c", "d"]);
        assert_eq!(py_splitlines("one\ntwo\n"), ["one", "two"]);
        assert_eq!(py_splitlines(""), Vec::<&str>::new());
        // A lone break ends an empty first line — `"\n".splitlines()` is
        // `[""]` in Python, checked against the interpreter.
        assert_eq!(py_splitlines("\n"), [""]);
        assert_eq!(
            py_splitlines("a\u{b}b\u{85}c\u{2028}d"),
            ["a", "b", "c", "d"]
        );
        // A break inside a line keeps the empty line it creates.
        assert_eq!(py_splitlines("a\n\nb"), ["a", "", "b"]);
    }
}
