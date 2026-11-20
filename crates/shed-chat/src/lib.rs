//! What the model's text *is*, before anyone speaks it.
//!
//! Ported from `utils/reply.py` and the reply half of `agent.py:5504-6202`: the
//! judgements about a reply's quality, the prose tool-call envelope the model
//! writes instead of calling a tool, the scanner that pulls the `reply` string
//! out of a JSON document while that document is still arriving, the sentence
//! splitter that decides when a sentence is safe to speak, and the publisher
//! that runs one through the other.
//!
//! ## Where this crate ends
//!
//! `PLAN-Rust-Clone.md` §8.2 splits the reply layer over three sub-slices, and
//! this crate is the point they meet:
//!
//! | Sub-slice | What it adds |
//! |---|---|
//! | **4f-1** (this) | the seven modules below — every rule, with `tests/` pinning each one against fixtures captured from the Python |
//! | 4f-2 | the agent's reply loop: conversation shaping, prompt variables, the engine call, the recovery ladder and the TTS handoff that drive these types |
//! | 4f-3 | the routes and the composition root that hand a request to that loop |
//!
//! So nothing here starts a thread, opens a file or binds a socket. The
//! scanner, the splitter and the publisher are state machines the *caller*
//! drives — 4f-2 owns the thread and the HTTP read that feeds them — and every
//! judgement is a function of the value handed to it. That is also why
//! [`RecentEvents`] takes `&mut self` and holds no lock: `utils/reply.py`'s
//! `_is_duplicate_event` wraps its list in a `threading.Lock` the caller owns,
//! and a lock is the caller's business, not the list's.
//!
//! ## Python's duck typing, spelled
//!
//! The source takes `object` and answers `str`/`bool`/`dict`/`None`. Here the
//! input is a [`serde_json::Value`] — the shape the messages array, the reply
//! document and the response `_meta` actually arrive as — and the answer is a
//! named type: [`String`], `bool`, `Option<Value>`, or [`ReplyDocument`] for
//! the two-key dict the source hands back. Two spellings are worth naming:
//!
//! - `str(value or "")` is [`shed_text::text_or_empty`], because `or` fires on
//!   every falsy value and not only on `None`.
//! - `.get(key, default)` over a mapping is a `match` on `Value` that answers
//!   the default for a non-object too, which is what Python's `.get` does when
//!   the receiver is a `str` or a `list`.
//!
//! ## The `re` patterns
//!
//! Every pattern is transcribed verbatim, flags included, and each module names
//! the line it came from. They are compiled with `fancy-regex` rather than
//! `regex` because three of them need a lookahead, named groups inside a
//! substitution callback, or a lazy quantifier — see the crate's `Cargo.toml`.
//! A pattern that fails to compile reads as absent, exactly as the source's own
//! `try: … except Exception:` around `markdown_to_ssml` reads a broken pattern
//! as "return the text unchanged"; none of them can fail here, and none of them
//! is allowed to take the reply down with it.

/// One `re` pattern from the source, compiled once per process.
///
/// Every pattern in this crate is transcribed verbatim, flags included, and
/// compiled on first use into an `OnceLock<Option<Regex>>`: Python compiles
/// once at import and caches forever, and a `OnceLock` is the same thing
/// without a module-level import side effect.
///
/// The `Option` is the point. `agent.py` wraps `markdown_to_ssml`'s
/// substitutions in `try: … except Exception:`, so a pattern that cannot
/// compile costs the substitution and never the reply. `None` is that outcome
/// spelled in Rust, and every caller here already reads `None` as "no match",
/// so the text comes back unchanged.
///
/// It defines a function rather than a `static` of its own because a `static`
/// initializer has to be a constant expression, and `OnceLock::get_or_init` is
/// deliberately not one: the whole purpose is to compile the pattern late.
///
/// Compiled with `fancy-regex` rather than the workspace's `regex`: this
/// crate's patterns use a lookahead (`space_after_sentences`), named groups
/// read inside a substitution callback (same), and a lazy quantifier
/// (`markdown_to_ssml`, `TOOL_CALL_ENVELOPE_PATTERN`), none of which the
/// backtracking-free `regex` crate accepts.
///
/// [`Regex`]: fancy_regex::Regex
macro_rules! pattern {
    ($name:ident, $src:expr) => {
        fn $name() -> Option<&'static fancy_regex::Regex> {
            static COMPILED: std::sync::OnceLock<Option<fancy_regex::Regex>> =
                std::sync::OnceLock::new();
            COMPILED.get_or_init(|| fancy_regex::Regex::new($src).ok()).as_ref()
        }
    };
}

mod document;
mod publisher;
mod quality;
mod reply_text;
mod splitter;
mod stream_scanner;
mod streamed;

pub use document::{
    parse_model_json_content, parse_model_reply_document, parse_structured_candidate,
    reply_without_prose_tool_calls, settle_prose_tool_calls, ReplyDocument,
};
pub use publisher::ReplyStreamPublisher;
pub use quality::{
    extract_reply_segments, latest_user_message_text, looks_incomplete,
    looks_like_action_teaser_reply, looks_like_generic_handoff_reply,
    looks_like_mctrader_action_request, looks_like_recovery_meta_reply,
    looks_like_task_stub_reply, looks_out_of_character_reply, looks_unusable_reply,
    merge_completed_reply, strip_tool_call_envelope, tool_calls_in_envelope, RecentEvents,
    REPLY_SEGMENT_MAX, TEASER_MAX_WORDS,
};
pub use reply_text::{markdown_to_ssml, normalize_reply_text};
pub use splitter::SentenceSplitter;
pub use stream_scanner::JsonReplyStreamScanner;
pub use streamed::{split_reply_after_stream, streamed_prefix_length, strip_streamed_prefix};

/// `utils/text.py`'s `COMMON_NON_TERMINAL_ABBREVIATIONS`.
///
/// One constant on purpose: `normalize_reply_text`'s sentence-spacing callback
/// and the splitter's abbreviation test both read it, and
/// `test_sentence_splitter.py`'s `test_abbreviation_boundary_uses_the_shared_rule`
/// is a statement that they agree. Lower-cased and without the trailing dot, as
/// the source stores them, so a caller compares a lower-cased word with its
/// dots stripped.
pub(crate) const COMMON_NON_TERMINAL_ABBREVIATIONS: [&str; 12] = [
    "dr", "e.g", "etc", "i.e", "jr", "mr", "mrs", "ms", "prof", "sr", "st", "vs",
];

/// Python's `str.strip()` with no argument.
///
/// The gap every copy of this helper names: Python treats `\u{1c}`–`\u{1f}` as
/// whitespace and [`char::is_whitespace`] does not, so a reply ending in one of
/// those would otherwise be trimmed here and kept there. `shed-store`
/// (`client_id.rs`), `shed-conversation` (`lib.rs`) and this crate each carry a
/// private copy because none of them may reach another's internals; a fourth
/// need moves it to `shed-text`, which all three are allowed to depend on.
pub(crate) fn py_strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}
