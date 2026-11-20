//! The text a reply *is*: `agent.py`'s `normalize_reply_text` and
//! `markdown_to_ssml`.
//!
//! `normalize_reply_text` is the last pass every reply makes before it is
//! stored, spoken or compared, which is why it is a pure function over one
//! value rather than a step of some pipeline: the recovery ladder, the stream
//! publisher and the routes all call it on text that arrived by three different
//! roads.
//!
//! `markdown_to_ssml` is Windows TTS only — markdown is how the entity is
//! prompted to write, and the voice is told to lean on a word instead of
//! shouting it.

use serde_json::Value;
use shed_text::text_or_empty;

use crate::COMMON_NON_TERMINAL_ABBREVIATIONS;

/// The six `re` expressions of these two functions, transcribed verbatim.
///
/// Python's flags become a `(?i)` / `(?is)` prefix, which is what those flags
/// mean to `re` — the whole pattern is case-insensitive — and keeps each
/// pattern a single string the reader can hold next to the source. They are
/// compiled through the crate's `pattern!`; the two that cannot compile are
/// the interesting ones, and each carries its own note below.

/// `utils/text.py:294`'s `PROTECTED_TEXT_PATTERN` (`re.IGNORECASE`): the spans
/// no normalisation may rewrite — a URL or an email address is data, not
/// something to say differently.
///
/// The client mirrors this one by hand (`SPEECH_PROTECTED_TEXT_PATTERN` in
/// `app-helpers.js`), so a drift here would let the two sides disagree about
/// which parts of a reply are words.
pattern!(
    protected_text,
    r"(?i)https?://[^\s,]+|www\.[^\s,]+|\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"
);
/// `agent.py`'s clause spacing: `([,;:])(?=(?:["'([{])?[A-Za-z])`.
///
/// The lookahead is why this crate uses `fancy-regex` — `regex`'s RE2 cannot
/// express it, and this one cannot simply be dropped: without it a comma that
/// the model wrote tight against the next word is spoken as one word.
pattern!(clause_spacing, r"([,;:])(?=(?:[\"'([{])?[A-Za-z])");
/// `agent.py`'s sentence spacing, with the three named groups the callback
/// reads (`token`, `punct`, `next`) kept as named groups for the same reason.
///
/// `next` matches a single letter, not a word: `re.sub` resumes after the
/// match, so a run of two stops — `No.Say it.` — only ever gains the spaces the
/// first one asks for. `test_reply_utils.py` pins that, which is why the group
/// is not widened to `[A-Za-z]+`.
pattern!(
    sentence_spacing,
    r"(?P<token>(?:__PROTECTED_TEXT_\d+__|\b[A-Za-z0-9][A-Za-z0-9.']{0,15}))(?P<punct>[.?!])(?:[ \t]+)?(?P<next>(?:[\"')\]]+[ \t]*)?[A-Za-z])"
);
/// `markdown_to_ssml`'s three substitutions, in the order it applies them.
///
/// All three are lazy (`+?`), which is the other RE2 exclusion: `"**a*b*c**"`
/// turns into two moderate emphases inside the literal asterisks the first
/// pattern could not match, and only laziness makes it so.
pattern!(strong_emphasis, r"\*\*([^*]+?)\*\*");
/// `markdown_to_ssml`'s second pass: `*text*` → moderate emphasis.
pattern!(moderate_emphasis, r"\*([^*]+?)\*");
/// `markdown_to_ssml`'s third pass: `_text_` → reduced pitch.
pattern!(softer_prosody, r"_([^_]+?)_");

/// `agent.py`'s `normalize_reply_text(value)`.
///
/// Python opens with `str(value or "")`, and the order matters: the falsy test
/// fires on `null`, `false`, `0`, `""`, `[]` and `{}` alike, so all six answer
/// `""` — while a truthy number still converts through `str()`.
/// [`shed_text::text_or_empty`] is that one expression spelled.
///
/// The passes run in this order, and the order is observable:
///
/// 1. literal `\n` / `\r` spellings become real breaks — a model that escapes
///    its own newlines is answered before real breaks are collapsed, so a
///    written `\r\n` cannot survive as a line ending;
/// 2. real CRLF and CR become LF, and three or more breaks become two;
/// 3. URLs and email addresses are lifted out into `__PROTECTED_TEXT_n__`
///    placeholders, so the two spacing passes below cannot see inside one;
/// 4. a comma, semicolon or colon tight against a word gains a space;
/// 5. a stop, question mark or exclamation point gains *two* spaces before the
///    next sentence — except after a known abbreviation, which gains one;
/// 6. the placeholders are put back, in the order they were taken out.
#[must_use]
pub fn normalize_reply_text(value: &Value) -> String {
    let text = text_or_empty(Some(value));
    if text.is_empty() {
        return String::new();
    }

    let written = expand_written_breaks(&text);
    let broken = collapse_breaks(&written);
    let (protected, placeholders) = protect_long_text(&broken);
    let spaced = space_before_clauses(&protected);
    let spaced = space_after_sentences(&spaced);
    let restored = restore_protected_text(&spaced, &placeholders);
    crate::py_strip(&restored).to_owned()
}

/// `agent.py`'s `markdown_to_ssml(text)`.
///
/// Empty text returns as it came. Otherwise the three passes run in order, and
/// a pattern that will not compile leaves the text as the passes before it left
/// it — the source's `try/except` returns the text it was handed, and the
/// fixture rows pin which pass each result came from.
#[must_use]
pub fn markdown_to_ssml(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let Some(strong) = strong_emphasis() else {
        return text.to_owned();
    };
    let text = strong
        .replace_all(text, r#"<emphasis level="strong">$1</emphasis>"#)
        .into_owned();
    let Some(moderate) = moderate_emphasis() else {
        return text;
    };
    let text = moderate
        .replace_all(&text, r#"<emphasis level="moderate">$1</emphasis>"#)
        .into_owned();
    let Some(softer) = softer_prosody() else {
        return text;
    };
    softer
        .replace_all(&text, r#"<prosody pitch="-7%">$1</prosody>"#)
        .into_owned()
}

/// The first pass: `text.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\r", "\n")`.
///
/// These are *written* breaks — a backslash and an `n` in the text — and the
/// CRLF spelling is replaced first so it does not become a stray backslash-r.
fn expand_written_breaks(text: &str) -> String {
    text.replace(r"\r\n", "\n").replace(r"\n", "\n").replace(r"\r", "\n")
}

/// Pass two, both halves: real `\r\n` and `\r` become `\n`, then a run of three
/// or more `\n` becomes two (`re.sub(r"\n{3,}", "\n\n")`).
///
/// The run is collapsed by hand rather than by a pattern because it is the one
/// pass here with nothing fancy about it, and a scan says in six lines what the
/// quantifier says in five characters.
fn collapse_breaks(text: &str) -> String {
    let flattened = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut collapsed = String::with_capacity(flattened.len());
    let mut run = 0_usize;
    for ch in flattened.chars() {
        if ch == '\n' {
            run += 1;
            if run <= 2 {
                collapsed.push(ch);
            }
        } else {
            run = 0;
            collapsed.push(ch);
        }
    }
    collapsed
}

/// Pass three: every protected span becomes `__PROTECTED_TEXT_{n}__`, where `n`
/// is how many have been taken out so far, and the span's trailing punctuation
/// stays outside the placeholder.
///
/// The punctuation is put back *outside* because the source returns
/// `f"{key}{trailing_punctuation}"` while storing only the peeled value: a URL
/// that ends a sentence keeps the stop where the sentence-spacing pass can see
/// it, and `"…example.com, and after it."` is read as prose around a URL rather
/// than as one long word.
///
/// The pairs are a `Vec` and not a `HashMap` because Python's dict is iterated
/// in insertion order when they are put back, and a placeholder restored out of
/// order could be rewritten by a later key.
fn protect_long_text(text: &str) -> (String, Vec<(String, String)>) {
    let mut placeholders: Vec<(String, String)> = Vec::new();
    let Some(pattern) = protected_text() else {
        return (text.to_owned(), placeholders);
    };
    let mut protected = String::with_capacity(text.len());
    let mut cursor = 0_usize;
    for found in pattern.find_iter(text) {
        // A span the engine cannot read ends the scan and leaves the rest of
        // the text untouched: `re.sub` would raise and take the whole
        // normalisation with it, which is the failure the `Option` here exists
        // to avoid.
        let Ok(found) = found else { break };
        if found.start() < cursor {
            continue;
        }
        protected.push_str(&text[cursor..found.start()]);
        cursor = found.end();
        let (value, trailing) = peel_trailing_punctuation(found.as_str());
        let key = format!("__PROTECTED_TEXT_{}__", placeholders.len());
        protected.push_str(&key);
        protected.push_str(trailing);
        placeholders.push((key, value));
    }
    protected.push_str(&text[cursor..]);
    (protected, placeholders)
}

/// The `while protected_value[-1] in ".,!?;:"` loop, which peels a run of stops
/// off the end of a protected span and keeps it in source order.
///
/// Returns the peeled value and the punctuation left over, as a borrowed slice
/// of the same text.
fn peel_trailing_punctuation(value: &str) -> (String, &str) {
    let mut end = value.len();
    while end > 0 {
        let Some(ch) = value[..end].chars().next_back() else {
            break;
        };
        if ".,!?;:".contains(ch) {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    (value[..end].to_owned(), &value[end..])
}

/// Pass four: `re.sub(r"([,;:])(?=…)", r"\1 ")`.
///
/// The lookahead is zero-width, so the replacement is the punctuation plus one
/// space and the word after it is never consumed — which is what lets
/// `"Wait,hold on;then,go"` gain three spaces from three separate matches.
fn space_before_clauses(text: &str) -> String {
    match clause_spacing() {
        Some(pattern) => pattern.replace_all(text, "$1 ").into_owned(),
        None => text.to_owned(),
    }
}

/// Pass five: two spaces after a sentence stop, one after an abbreviation.
///
/// Two, because the source is preparing text for a voice and for a reader of
/// the transcript at once, and the double stop is how the transcript shows a
/// sentence boundary the model wrote without a space. The abbreviation list is
/// [`crate::COMMON_NON_TERMINAL_ABBREVIATIONS`] — the very one the sentence
/// splitter reads, which is what `test_abbreviation_boundary_uses_the_shared_rule`
/// is a statement about.
fn space_after_sentences(text: &str) -> String {
    let Some(pattern) = sentence_spacing() else {
        return text.to_owned();
    };
    pattern
        .replace_all(text, |caps: &fancy_regex::Captures<'_, str>| {
            let token = caps.name("token").map_or("", |found| found.as_str());
            let punctuation = caps.name("punct").map_or("", |found| found.as_str());
            let next = caps.name("next").map_or("", |found| found.as_str());
            let lowered = token.to_lowercase();
            let spacing = if punctuation == "."
                && COMMON_NON_TERMINAL_ABBREVIATIONS
                    .iter()
                    .any(|abbrev| *abbrev == lowered)
            {
                " "
            } else {
                "  "
            };
            format!("{token}{punctuation}{spacing}{next}")
        })
        .into_owned()
}

/// Pass six: `for key, original_value in placeholders.items(): normalized.replace(key, original_value)`.
///
/// Every occurrence, in insertion order, exactly as Python spells it — a model
/// that quotes the same URL twice has it protected once and restored at both
/// places.
fn restore_protected_text(text: &str, placeholders: &[(String, String)]) -> String {
    let mut restored = text.to_owned();
    for (key, original_value) in placeholders {
        restored = restored.replace(key, original_value);
    }
    restored
}
