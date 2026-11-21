//! Sentence boundaries in a reply that is still arriving.
//!
//! Port of `SentenceSplitter` from `agent.py:5819-5939`, plus the
//! `_SPLITTER_MARKUP_PAIRS`, `_SPLITTER_TERMINATORS`, `_SPLITTER_SOFT_BREAKS` and
//! `_DOTTED_ACRONYM_RE` it reads.
//!
//! ## Why a sentence has to be *earned*
//!
//! The publisher hands every sentence it releases to the text-to-speech backend
//! as its own request, and one spoken out of order cannot be taken back. So a
//! boundary needs a terminator, then whitespace, then proof that the text so far
//! can stand alone: `Mr.` is not a sentence, `3.` is a number, `{tone` is half a
//! brace, and a run too short to be worth a clip of its own ("Ha. Yes.") waits
//! for company.
//!
//! ## Characters, not bytes
//!
//! Every index here is a *character* index, because Python indexes strings by
//! character and the boundaries are compared against `len(buffer)`. The buffer is
//! therefore a `Vec<char>`, so slicing it by index is the same operation the
//! Python performs and [`chars_to_string`] is the only place bytes reappear.
//!
//! ## The two bounds
//!
//! `MIN_SENTENCE_CHARS` (12) is the shortest sentence that may be released.
//! `MAX_SENTENCE_CHARS` (240) is the window [`SentenceSplitter::forced_boundary`]
//! cuts inside when a buffer has grown past it without ever ending: a model that
//! never writes a period still gets its reply spoken in pieces rather than in one
//! four-thousand-character shout.

use crate::{py_digit, py_strip, py_whitespace_char, COMMON_NON_TERMINAL_ABBREVIATIONS};

/// Markup the client and `segment_text_by_brackets` both read as control text —
/// a split inside an unbalanced run of it would change what gets spoken.
///
/// `*` and `_` are pairs of *one* character each, so each is checked for parity
/// on its own: `_` does not close a `*`, and markdown emphasis written either way
/// has to be closed the way it was opened. Curly quotes are not in this list.
const MARKUP_PAIRS: [(&str, &str); 5] =
    [("{", "}"), ("[", "]"), ("(", ")"), ("*", "*"), ("_", "_")];

/// `.!?…` — the characters that may end a sentence.
const TERMINATORS: [char; 4] = ['.', '!', '?', '…'];

/// `;:—–` — a trailing run of these is a soft pause, the last place to cut a
/// sentence that has grown past the maximum without reaching a full stop.
///
/// A comma is deliberately absent: cutting there hands the listener a clip
/// boundary where the sentence has not ended, and the gap between clips is heard
/// as an over-long pause after the comma. Such a sentence falls back to the
/// word-boundary cut instead — the same place the client's own chunker cuts.
const SOFT_BREAKS: [char; 4] = [';', ':', '—', '–'];

// `agent.py:5809`'s `_DOTTED_ACRONYM_RE`:
// `re.compile(r"^(?:[A-Za-z]\.){2,}[A-Za-z]?$")` — two or more single letters
// separated by dots: `U.S`, `e.g`, `a.m`.
pattern!(dotted_acronym, r"^(?:[A-Za-z]\.){2,}[A-Za-z]?$");

// `re.search(r"[A-Za-z][A-Za-z.]*$", buffer[:index + 1])` — the trailing run of
// letters and dots the period sits at the end of.
pattern!(trailing_token, r"[A-Za-z][A-Za-z.]*$");

/// Chop streamed reply text into speakable sentences.
///
/// Only text that is safe to speak on its own is released. Anything ambiguous
/// waits for more input: a split inside `{tone}`, `[scene]`, `*action*` or
/// `_emphasis_` markup, inside `3.5`, inside `Mr.`, or inside a sentence too
/// short to stand alone.
#[derive(Clone, Debug, Default)]
pub struct SentenceSplitter {
    buffer: Vec<char>,
}

impl SentenceSplitter {
    /// The shortest thing this will release as a sentence.
    pub const MIN_SENTENCE_CHARS: usize = 12;
    /// The longest a sentence may grow before one is forced out of the buffer.
    pub const MAX_SENTENCE_CHARS: usize = 240;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a chunk and return every sentence it completed.
    pub fn feed(&mut self, delta: Option<&str>) -> Vec<String> {
        self.buffer.extend(delta.unwrap_or_default().chars());
        self.extract()
    }

    /// Release whatever is left, which is the end of the reply.
    pub fn flush(&mut self) -> Vec<String> {
        let remainder = py_strip(&chars_to_string(&self.buffer)).to_owned();
        self.buffer.clear();
        if remainder.is_empty() {
            Vec::new()
        } else {
            vec![remainder]
        }
    }

    /// Cut sentences off the front of the buffer until none can be cut.
    ///
    /// Three exits, in the source's order: a boundary, a buffer that has not
    /// grown past the maximum, and a forced cut with nothing to cut at.
    fn extract(&mut self) -> Vec<String> {
        let mut sentences = Vec::new();
        loop {
            if let Some(boundary) = self.next_boundary() {
                sentences.extend(cut(&mut self.buffer, boundary));
                continue;
            }

            if self.buffer.len() <= Self::MAX_SENTENCE_CHARS {
                return sentences;
            }

            let Some(forced) = self.forced_boundary() else {
                return sentences;
            };
            let sentence = trim_front(&self.buffer, forced);
            if sentence.is_empty() {
                return sentences;
            }
            self.buffer.drain(..forced);
            sentences.push(sentence);
        }
    }

    /// The first character index the buffer may be cut at, or `None`.
    ///
    /// The scan runs from the very beginning, and stops at the first terminator
    /// whose run reaches the end of the buffer: whatever follows a terminator has
    /// not arrived yet, and end-of-buffer is never a safe place to cut — so no
    /// later terminator gets a look-in until it does.
    fn next_boundary(&self) -> Option<usize> {
        let buffer = &self.buffer;
        for index in 0..buffer.len() {
            if !TERMINATORS.contains(&buffer[index]) {
                continue;
            }
            let mut end = index + 1;
            while end < buffer.len() && TERMINATORS.contains(&buffer[end]) {
                end += 1;
            }
            if end >= buffer.len() {
                return None;
            }
            if !py_whitespace_char(buffer[end]) {
                continue;
            }
            // A terminator run of one character is only worth the cut if it is a
            // real ending; `!!` and `?!` are endings whatever they follow.
            if end - index < 3 && !self.is_sentence_end(index) {
                continue;
            }
            if end < Self::MIN_SENTENCE_CHARS {
                continue;
            }
            if !markup_balanced(&buffer[..end]) {
                continue;
            }
            return Some(end);
        }
        None
    }

    /// `True` when `buffer[index]` ends a sentence, which is most of the class's
    /// care in one function.
    fn is_sentence_end(&self, index: usize) -> bool {
        // A single "." is also how an abbreviation, an initial or a decimal point
        // is written, so it only ends a sentence when the token before it is not
        // one of those.
        self.buffer[index] != '.' || !self.is_non_terminal_period(index)
    }

    fn is_non_terminal_period(&self, index: usize) -> bool {
        let buffer = &self.buffer;
        if buffer[index] != '.' {
            return false;
        }
        // `3.5`: the point is inside a number when digits sit on both sides of
        // it. A digit after a period that follows anything else is a new sentence
        // that happens to start with a figure.
        let following = index + 1;
        if following < buffer.len()
            && py_digit(buffer[following])
            && index > 0
            && py_digit(buffer[index - 1])
        {
            return true;
        }
        let token = token_before(buffer, index);
        if token.is_empty() {
            return false;
        }
        if COMMON_NON_TERMINAL_ABBREVIATIONS.contains(&token.as_str()) {
            return true;
        }
        // Dotted acronyms and initials — "U.S.", "e.g.", "a.m." — are not in the
        // shared abbreviation list, and adding them there would change how every
        // other caller reads text.
        dotted_acronym().is_some_and(|regex| regex.is_match(&token).unwrap_or(false))
    }

    /// A place to cut a reply that has grown past `MAX_SENTENCE_CHARS` without
    /// ending: the last soft break in the window, then the last whitespace, then
    /// nothing at all.
    ///
    /// Scanning backwards is the whole design — the longest sentence that still
    /// reads as one sentence. The soft-break pass requires the break itself to be
    /// inside balanced markup and cuts *after* it; the whitespace pass requires
    /// only the text before it to be balanced and cuts *at* it, so the space ends
    /// up stripped off the front of the next sentence.
    fn forced_boundary(&self) -> Option<usize> {
        let limit = Self::MAX_SENTENCE_CHARS;
        let window = &self.buffer[..self.buffer.len().min(limit)];
        for index in (1..window.len()).rev() {
            if !SOFT_BREAKS.contains(&window[index]) {
                continue;
            }
            if !markup_balanced(&window[..=index]) {
                continue;
            }
            return Some(index + 1);
        }
        for index in (1..window.len()).rev() {
            if py_whitespace_char(window[index]) && markup_balanced(&window[..index]) {
                return Some(index);
            }
        }
        None
    }
}

/// The word the `.` sits at the end of, lower-cased and with its dots stripped —
/// the form both the abbreviation list and the acronym pattern are read in.
///
/// Empty answers "the pattern did not match", which is what `if not token_match`
/// means in the source.
fn token_before(buffer: &[char], index: usize) -> String {
    let prefix = chars_to_string(&buffer[..=index]);
    trailing_token()
        .and_then(|regex| regex.find(&prefix).ok().flatten())
        .map(|found| {
            found
                .as_str()
                .to_lowercase()
                .trim_end_matches('.')
                .to_owned()
        })
        .unwrap_or_default()
}

/// `buffer[..index]` has no unmatched or crossed markup.
fn markup_balanced(text: &[char]) -> bool {
    MARKUP_PAIRS.iter().all(|(opening, closing)| {
        let open = opening.chars().next().unwrap_or('\0');
        let close = closing.chars().next().unwrap_or('\0');
        let open_count = text.iter().filter(|ch| **ch == open).count();
        if open == close {
            // `*action*` and `_emphasis_` use the same character twice, so the
            // only thing to check is that the run is closed.
            open_count % 2 == 0
        } else {
            open_count == text.iter().filter(|ch| **ch == close).count()
        }
    })
}

/// Cut `index` characters off the front of the buffer, keeping a non-empty
/// sentence: the source's `sentence = buffer[:n].strip()` and its `if sentence`.
fn cut(buffer: &mut Vec<char>, index: usize) -> Option<String> {
    let sentence = trim_front(buffer, index);
    buffer.drain(..index);
    if sentence.is_empty() {
        None
    } else {
        Some(sentence)
    }
}

/// `buffer[:index].strip()` without touching the buffer.
fn trim_front(buffer: &[char], index: usize) -> String {
    py_strip(&chars_to_string(&buffer[..index])).to_owned()
}

fn chars_to_string(chars: &[char]) -> String {
    chars.iter().collect()
}
