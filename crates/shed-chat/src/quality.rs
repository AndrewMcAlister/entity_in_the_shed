//! Is this reply good enough to speak?
//!
//! Every function here answers one question about a reply — is it truncated, is
//! it a stub, does it break character, does it duplicate the event we just read
//! out — and none of them change what the model said. They are the guards the
//! agent runs *before* a reply reaches the browser, and they all fail the same
//! way: the caller keeps the reply and takes a different route with it, so a
//! false positive costs a retry rather than a lost answer. That is why several
//! of them are deliberately loose, and why each one is a judgement returning a
//! `bool` rather than an operation returning a `Result`.
//!
//! The source is the whole of `utils/reply.py` in the reference app: the
//! constants at 16, 25 and 28-46, the first five judgements at 49-167,
//! `merge_completed_reply` at 170-200, `extract_reply_segments` at 203-233, the
//! prose tool-call envelope at 241-301, the last four judgements at 304-381 and
//! `_is_duplicate_event` at 384-389. `agent.py:157` imports all fourteen; the
//! private helpers (`_TEASER_PATTERNS`, `_tool_calls_in_envelope_body`,
//! `_normalize_tool_call`) ride along here as private functions.

use std::collections::VecDeque;
use std::sync::OnceLock;

use fancy_regex::Regex;
use serde_json::{Map, Value};
use shed_text::{py_str, text_or_empty};

use crate::{collapse_whitespace, py_strip, py_word_count};

/// How many separate bubbles one turn may produce.
///
/// Small on purpose: the point is genuinely separate beats, not splitting every
/// answer into fragments, which reads as a stutter rather than an entity
/// talking.
pub const REPLY_SEGMENT_MAX: usize = 4;

/// A reply announcing work instead of reporting a result is short by nature: the
/// announcement is the whole message.
///
/// Real answers that happen to contain a phrase like "checking" are long, and
/// the limit keeps the teaser patterns from firing on those. That matters
/// because the consumer at `agent.py:resolve_followup_action_request` re-issues
/// the prior request when it sees a teaser — a false positive there is a wrong
/// turn, not a wasted call.
pub const TEASER_MAX_WORDS: usize = 40;

/// The teaser phrases, grouped by frame rather than by verb so the list stays
/// readable.
///
/// The "take a look" group is the axis that was missing when this was first
/// written: the reported failure was "Let me take a closer look.", which
/// announced work and never reported a result, and none of the retrieval verbs
/// above it match it because "take" is not one of them.
const ACTION_TEASER_PATTERNS: [&str; 12] = [
    r"\blet me\s+(?:fetch|get|check|pull|retrieve|look up)\b",
    r"\bi(?:'ll| will| shall)\s+(?:fetch|get|check|pull|retrieve|look up)\b",
    r"\bfetching\b",
    r"\bchecking\b",
    r"\blet me\s+(?:take|have)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\bi(?:'ll| will| shall)\s+(?:take|have)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\b(?:taking|having)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\blet me\s+(?:look|see|find out|dig into|examine|investigate|review|go over|work through)\b",
    r"\bi(?:'ll| will| shall)\s+(?:look|see|find out|dig into|examine|investigate|review|go over|work through)\b",
    r"\b(?:looking|digging)\s+into\b",
    r"\bsee what i can (?:find|do|dig up)\b",
    r"\b(?:investigating|examining)\b",
];

/// Words that make a message about this app's trading desk.
const MCTRADER_CONTEXT_TOKENS: [&str; 7] = [
    "mctrader",
    "mc trader",
    "matt trader",
    "trader",
    "broker",
    "watchlist",
    "portfolio",
];

/// Words that make it a request about something on that desk.
///
/// Deliberately wider than the context list — "position", "balance", "status" —
/// because a message that names the desk *and* one of these is asking for a
/// number, and the caller routes numbers away from the model.
const MCTRADER_INTENT_TOKENS: [&str; 19] = [
    "position",
    "positions",
    "account",
    "balance",
    "equity",
    "status",
    "running",
    "watchlist",
    "symbol",
    "symbols",
    "ticker",
    "analyse",
    "analyze",
    "analysis",
    "broker",
    "exchange",
    "start trader",
    "stop trader",
    "restart trader",
];

/// Lines of verse rarely end in terminal punctuation, but only a *short* block
/// reads as a poem rather than a truncated answer.
const POEM_INTROS: [&str; 9] = [
    "here's a short poem",
    "here is a short poem",
    "here's your short poem",
    "here is your short poem",
    "here's a concise poem",
    "here is a concise poem",
    "here's a poem",
    "here is a poem",
    "here is the poem",
];

/// A request to *make* something, which is what a one-line "I'll write that
/// now" answers instead of the thing itself. Both groups must hit.
const GENERATIVE_REQUEST_PATTERNS: [&str; 2] = [
    r"\b(write|draft|compose|create|make|generate)\b",
    r"\b(poem|story|letter|email|speech|song|essay|script|paragraph|list)\b",
];

/// The shape of a stub answer to such a request: a promise, no content.
const TASK_STUB_REPLY_PATTERNS: [&str; 3] = [
    r"^(i will|i'll|i shall)\b",
    r"^(here(?:'s| is)|certainly|sure|alright|okay|ok)\b",
    r"\b(write|draft|compose|create|make)\b.*\b(for you|now)\b",
];

/// Punctuation that means the reply went on to say something real. A stub stops
/// before any of it.
const CONTENT_MARKERS: [&str; 5] = ["\"", "'", ",", ";", ":"];

/// Replies that are the app's own failure text coming back around.
const UNUSABLE_REPLIES: [&str; 6] = [
    "i could not generate a response",
    "i couldn't generate a response",
    "i cannot generate a response",
    "unable to generate a response",
    "i'm unable to generate a response",
    "i am unable to generate a response",
];

/// What the source strips around a reply before comparing it to that table:
/// whitespace plus the quotation marks and terminal punctuation a wrapper may
/// have added.
const UNUSABLE_TRIM_CHARS: &str = " \t\r\n\"'`.,!?";

/// The assistant describing what it is rather than being it.
const OUT_OF_CHARACTER_PHRASES: [&str; 4] = [
    "as an ai",
    "as an ai assistant",
    "i do not possess a personal identity",
    "i don't possess a personal identity",
];

/// The assistant narrating its own recovery instead of answering.
///
/// These are the app's own error strings coming back in the model's voice — the
/// model has read a recovery note and is repeating it — so they are quoted in
/// full rather than patterned.
const RECOVERY_META_PHRASES: [&str; 12] = [
    "there was no content in the previous reply",
    "there was no incomplete reply to continue from",
    "there was no reply text",
    "there was nothing to continue from",
    "previous message contained no text at all",
    "empty object",
    "empty reply field",
    "complete, albeit empty, json object",
    "only an empty reply field in the json",
    "system returned an empty object",
    "continue from nothing",
    "did not intend to cut you short",
];

/// A reply that hands the turn back instead of answering it.
///
/// This is the character's own register — "pray tell", "i remain at your
/// disposal" — which is why the list is phrases rather than a generic shape: the
/// app's entity is allowed to be florid, and only these openings are its
/// ways of not answering.
const HANDOFF_MARKERS: [&str; 7] = [
    "tell me your question or your task",
    "you're free to ask whatever you please",
    "i remain at your disposal",
    "pray tell",
    "without interruption",
    "would you like me to assist with something specific",
    "i'm ready to help with automation, spreadsheets, or any factual questions you have",
];

// Terminal punctuation with nothing but whitespace after it — the shape a
// sentence ends in and a truncation does not. The source writes
// `[.!?\"')\]\u201d]`, so the class holds `.`, `!`, `?`, both quotes, `)`, `]`
// and the closing curly quote.
pattern!(terminal_punctuation, r#"[.!?"')\]\u{201d}]\s*$"#);

// A completion that opens with a capital or a quote is starting a new sentence,
// which is not the same thing as repeating the tail of what it continues. The
// source writes this `^[A-Z\"']` with `re.match`; the backslash there is
// Python's string escape, not the regex's, so the class is written directly as
// the three characters it holds.
pattern!(completion_opens_sentence, r#"^[A-Z"']"#);

// The prompt asks for a tool call as a JSON field, but a model sometimes writes
// the envelope as prose instead — `{tool_calls}{"tool": …}{/tool_calls}` —
// borrowing the tone-tag syntax the same prompt teaches a few lines below the
// schema. The tag has to literally name `tool_calls` for this to match, which is
// what keeps a real tone tag from ever being read as an envelope and stripped
// out of what the user hears.
pattern!(
    tool_calls_envelope,
    r"(?is)\{\s*tool_calls\s*\}(?P<body>.*?)\{\s*/\s*tool_calls\s*\}"
);

// An envelope that was opened and never closed leaves its tag behind. A tag
// naming `tool_calls` is markup wherever it appears, so it always goes; what it
// was carrying is left as written, because guessing where an unterminated
// envelope ends would sometimes delete a real answer.
pattern!(tool_calls_tag, r"(?i)\{\s*/?\s*tool_calls\s*\}");

// The question a handoff answers when it has none of the character's own
// markers.
pattern!(
    handoff_question,
    r"\bhow may i assist you(?:\s+(?:today|now))?\b"
);

// A bare greeting is not a question, so a handoff is the right answer to it. The
// source tests this with `re.fullmatch`, which is `^(?:…)$` here; the text handed
// to it has already been stripped and collapsed, so `$` has no trailing
// whitespace to argue about.
pattern!(
    greeting_only,
    r"^(?:hello|hi|hey|good (?:day|morning|afternoon|evening))(?:[.!?])*$"
);

/// `re.search(pattern, text)`, where a pattern that failed to compile, or a
/// match the engine gave up on, searches as no match at all.
fn searches(pattern: Option<&Regex>, text: &str) -> bool {
    pattern.is_some_and(|regex| regex.find(text).unwrap_or_default().is_some())
}

/// `any(re.search(p, text) for p in patterns)`.
fn any_search(patterns: &[Option<Regex>], text: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| searches(pattern.as_ref(), text))
}

/// `all(re.search(p, text) for p in patterns)`.
fn all_search(patterns: &[Option<Regex>], text: &str) -> bool {
    patterns
        .iter()
        .all(|pattern| searches(pattern.as_ref(), text))
}

/// The compiled form of a phrase table, once.
fn compiled(sources: &'static [&'static str]) -> &'static [Option<Regex>] {
    static TEASERS: OnceLock<Vec<Option<Regex>>> = OnceLock::new();
    static REQUESTS: OnceLock<Vec<Option<Regex>>> = OnceLock::new();
    static STUBS: OnceLock<Vec<Option<Regex>>> = OnceLock::new();
    let cell = if std::ptr::eq(sources.as_ptr(), ACTION_TEASER_PATTERNS.as_ptr()) {
        &TEASERS
    } else if std::ptr::eq(sources.as_ptr(), GENERATIVE_REQUEST_PATTERNS.as_ptr()) {
        &REQUESTS
    } else if std::ptr::eq(sources.as_ptr(), TASK_STUB_REPLY_PATTERNS.as_ptr()) {
        &STUBS
    } else {
        unreachable!("every phrase table in this module has its own cell")
    };
    cell.get_or_init(|| sources.iter().map(|src| Regex::new(src).ok()).collect())
}

/// A reply that promises work rather than reporting a result: "Let me check on
/// that."
///
/// Only short replies are judged — the boundary is a word count, not a phrase —
/// and the text has its whitespace runs collapsed first so a reply broken over
/// lines is matched by phrase rather than by line.
#[must_use]
pub fn looks_like_action_teaser_reply(reply_text: &str) -> bool {
    let text = collapse_whitespace(&py_strip(reply_text).to_lowercase());
    if text.is_empty() || py_word_count(&text) > TEASER_MAX_WORDS {
        return false;
    }
    any_search(compiled(&ACTION_TEASER_PATTERNS), &text)
}

/// A user text that asks about something on the trading desk: one context word
/// *and* one intent word, both substring-matched on the lowered text.
#[must_use]
pub fn looks_like_mctrader_action_request(user_text: &str) -> bool {
    let text = collapse_whitespace(&py_strip(user_text).to_lowercase());
    if text.is_empty() {
        return false;
    }
    let has_trader_context = MCTRADER_CONTEXT_TOKENS
        .iter()
        .any(|token| text.contains(token));
    let has_action_intent = MCTRADER_INTENT_TOKENS
        .iter()
        .any(|token| text.contains(token));
    has_trader_context && has_action_intent
}

/// Whether the model stopped of its own accord before finishing.
///
/// The source tests the raw reply for falsiness first, so an empty one is
/// incomplete whatever follows, and it reads `_meta.done_reason` from the
/// response details: a turn cut off by the token limit is incomplete however
/// complete the text looks. A `done_reason` present with JSON `null` reads as
/// the string `"None"`, because the source calls `str()` on whatever it got,
/// which is not the `""` a missing key gives.
///
/// A `_meta` that is neither an object nor null raises `AttributeError` in
/// Python. Here it reads as absent, the one place in this crate where a source
/// exception cannot be reproduced; no shipped model response holds one.
#[must_use]
pub fn looks_incomplete(reply_text: &str, response_details: Option<&Value>) -> bool {
    if reply_text.is_empty() {
        return true;
    }

    // `(response_meta or {}).get("done_reason", "")`: an absent or null `_meta`
    // reads as an empty mapping.
    let done_reason = response_details
        .filter(|details| details.is_object())
        .and_then(|details| details.get("_meta"))
        .and_then(Value::as_object)
        .map(|meta| meta.get("done_reason").map_or_else(String::new, py_str))
        .unwrap_or_default();
    if py_strip(&done_reason.to_lowercase()) == "length" {
        return true;
    }

    let stripped = py_strip(reply_text);
    if stripped.ends_with(':') || stripped.ends_with("...") {
        return true;
    }
    if stripped.chars().count() >= 60
        && stripped.contains('\n')
        && !searches(terminal_punctuation(), stripped)
    {
        return true;
    }

    let lowered = stripped.to_lowercase();
    POEM_INTROS.iter().any(|phrase| lowered.contains(phrase)) && py_word_count(stripped) <= 25
}

/// The text of the most recent user message, `""` when the history has none.
///
/// `str(message.get("content", ""))`, so a message whose content is absent reads
/// as empty and one whose content is JSON `null` reads as `"None"`. An entry
/// that is not an object raises in Python and is skipped here, as in
/// [`looks_incomplete`].
#[must_use]
pub fn latest_user_message_text(messages: &[Value]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|message| {
            let entry = message.as_object()?;
            if entry.get("role").is_some_and(|role| role == "user") {
                Some(py_strip(&entry.get("content").map_or_else(String::new, py_str)).to_owned())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// A one-line promise to write something, where the writing never came.
///
/// The user has to have asked for a *thing* to be made — one pattern from each
/// of the two groups — and the reply has to be a single short line that reads as
/// a promise and holds none of the [`CONTENT_MARKERS`]. That last test is what
/// separates "I'll write that now" from a one-line answer that happens to start
/// "Sure".
#[must_use]
pub fn looks_like_task_stub_reply(user_text: &str, reply_text: &str) -> bool {
    let user_lower = py_strip(user_text).to_lowercase();
    let reply = py_strip(reply_text).to_owned();
    if user_lower.is_empty() || reply.is_empty() {
        return false;
    }
    if !all_search(compiled(&GENERATIVE_REQUEST_PATTERNS), &user_lower) {
        return false;
    }
    if reply.contains('\n') || py_word_count(&reply) > 18 {
        return false;
    }
    if !any_search(compiled(&TASK_STUB_REPLY_PATTERNS), &reply.to_lowercase()) {
        return false;
    }
    !CONTENT_MARKERS.iter().any(|marker| reply.contains(marker))
}

/// Stitch a continuation onto the reply it continues.
///
/// The model is asked to carry on from where it stopped and often repeats a
/// little of what it already said, so the longest overlap between the tail of the
/// partial reply and the head of the completion is cut out — up to 240 characters
/// of it, which is what the source caps at. A completion that starts a new
/// sentence is kept as it is, because a capital after a full stop is not a
/// repetition, and a completion that turns out to be nothing but overlap returns
/// the partial reply unchanged.
#[must_use]
pub fn merge_completed_reply(partial_reply: &str, completion: &str) -> String {
    let completion_stripped = py_strip(completion).to_owned();
    if completion_stripped.is_empty() {
        return py_strip(partial_reply).to_owned();
    }

    let partial_stripped = py_strip(partial_reply).to_owned();
    if partial_stripped.is_empty() {
        return completion_stripped;
    }
    if completion_stripped.starts_with(&partial_stripped) {
        return completion_stripped;
    }

    let partial: Vec<char> = partial_stripped.chars().collect();
    let completion_chars: Vec<char> = completion_stripped.chars().collect();
    let max_overlap = partial.len().min(completion_chars.len()).min(240);
    let mut overlap = 0;
    for size in (1..=max_overlap).rev() {
        if completion_chars[..size] == partial[partial.len() - size..] {
            overlap = size;
            break;
        }
    }

    if overlap == 0 && searches(completion_opens_sentence(), &completion_stripped) {
        return completion_stripped;
    }

    let suffix: String = completion_chars[overlap..].iter().collect();
    if suffix.is_empty() {
        return partial_stripped;
    }

    // `str.isalnum()` on each edge — two words need a space between them, a word
    // and punctuation do not, and an empty edge is not alphanumeric.
    let joiner = if partial.last().copied().is_some_and(char::is_alphanumeric)
        && suffix.chars().next().is_some_and(char::is_alphanumeric)
    {
        " "
    } else {
        ""
    };
    py_strip(&format!("{partial_stripped}{joiner}{suffix}")).to_owned()
}

/// Split a model's `replies` field into clean, capped segments.
///
/// Returns an empty vector when there is nothing usable, which is the caller's
/// signal to fall back to the single `reply` string. One turn may legitimately
/// produce several distinct beats — a preface and a result — and they should
/// reach the browser as separate bubbles rather than one growing one.
///
/// Purely textual: it strips, drops non-strings and empties, and caps the count.
/// It deliberately does *not* normalise, because `normalize_reply_text` lives
/// with the app and this crate must not depend on it.
#[must_use]
pub fn extract_reply_segments(value: &Value) -> Vec<String> {
    let candidates: Vec<&Value> = match value {
        Value::String(_) => vec![value],
        Value::Array(items) => items.iter().collect(),
        _ => return Vec::new(),
    };

    let mut segments = Vec::new();
    for item in candidates {
        let Value::String(text) = item else {
            continue;
        };
        let cleaned = py_strip(text);
        if cleaned.is_empty() {
            continue;
        }
        segments.push(cleaned.to_owned());
        if segments.len() >= REPLY_SEGMENT_MAX {
            break;
        }
    }
    segments
}

/// Remove any tool-call envelope a model wrote as prose rather than as JSON.
///
/// Each envelope becomes a single space, so "now.{…}" reads "now. " rather than
/// "now.". Callers normalise. A reply with no envelope comes back byte-identical
/// apart from that rule, which is what lets the caller use this on every reply.
#[must_use]
pub fn strip_tool_call_envelope(text: &str) -> String {
    let without_envelope = replace_all(tool_calls_envelope(), text, " ");
    replace_all(tool_calls_tag(), &without_envelope, " ")
}

/// The tool calls carried in a prose envelope, or an empty vector when there is
/// none to read.
///
/// Empty when there is no envelope, when its body is not JSON, and when it holds
/// no call with a name. The caller strips the envelope either way, so a body this
/// cannot read costs the tool call and never the reply.
#[must_use]
pub fn tool_calls_in_envelope(text: &str) -> Vec<Value> {
    let mut calls = Vec::new();
    let Some(regex) = tool_calls_envelope() else {
        return calls;
    };
    for matched in regex.captures_iter(text) {
        // An engine that gives up partway stops the scan, as the source's
        // `for` loop does when `finditer` raises.
        let Ok(captures) = matched else { break };
        let body = captures.name("body").map_or("", |group| group.as_str());
        calls.extend(tool_calls_in_envelope_body(body));
    }
    calls
}

/// One `re.sub` where a pattern that failed to compile, or an engine that gave up
/// partway, leaves the text exactly as it was. `NoExpand` because every
/// replacement here is a literal; Python's `re.sub` would expand `\1`, but none
/// of these replacements hold a backslash, so the two agree.
fn replace_all(pattern: Option<&Regex>, text: &str, with: &str) -> String {
    pattern
        .and_then(|regex| {
            regex
                .try_replacen(text, 0, fancy_regex::NoExpand(with))
                .ok()
        })
        .map_or_else(|| text.to_owned(), |replaced| replaced.into_owned())
}

/// Read one envelope body: a single call object, or the array the schema itself
/// shows.
fn tool_calls_in_envelope_body(body: &str) -> Vec<Value> {
    let Ok(parsed) = serde_json::from_str::<Value>(py_strip(body)) else {
        return Vec::new();
    };
    let items: Vec<&Value> = match parsed {
        Value::Object(_) => vec![&parsed],
        Value::Array(ref items) => items.iter().collect(),
        _ => return Vec::new(),
    };
    items
        .into_iter()
        .filter_map(normalize_tool_call)
        .collect::<Vec<_>>()
}

/// One call in the shape the tool runner expects, or `None` when it names no
/// tool.
///
/// The tool runner indexes into each call, so anything that is not an object
/// naming a tool is dropped here rather than reaching it as an `AttributeError`.
/// `str(item.get("tool") or "")`, so a numeric name arrives as its digits and a
/// `null` one as no name at all.
fn normalize_tool_call(item: &Value) -> Option<Value> {
    let entry = item.as_object()?;
    let tool_name = py_strip(&text_or_empty(entry.get("tool"))).to_owned();
    if tool_name.is_empty() {
        return None;
    }
    let args = entry
        .get("args")
        .filter(|args| args.is_object())
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut call = Map::new();
    call.insert("tool".to_owned(), Value::String(tool_name));
    call.insert("args".to_owned(), args);
    Some(Value::Object(call))
}

/// A reply the app should not speak: empty, or one of its own failure lines
/// arriving back as if it were an answer.
///
/// The comparison trims whitespace *and* the quotation marks and terminal
/// punctuation a wrapper may have added, so `"I could not generate a response."`
/// is as unusable as the bare sentence.
#[must_use]
pub fn looks_unusable_reply(reply_text: &str) -> bool {
    let stripped = py_strip(reply_text);
    if stripped.is_empty() {
        return true;
    }
    let lowered = py_strip_chars(&stripped.to_lowercase(), UNUSABLE_TRIM_CHARS);
    UNUSABLE_REPLIES.contains(&lowered.as_str())
}

/// Python's `str.strip(chars)` — trim exactly the characters listed, nothing
/// else. `py_strip` cannot be used here: this table is not whitespace.
fn py_strip_chars(text: &str, chars: &str) -> String {
    let is_trim = |ch: char| chars.chars().any(|candidate| candidate == ch);
    text.trim_start_matches(is_trim)
        .trim_end_matches(is_trim)
        .to_owned()
}

/// The assistant describing what it is rather than being it.
#[must_use]
pub fn looks_out_of_character_reply(reply_text: &str) -> bool {
    let lowered = py_strip(reply_text).to_lowercase();
    OUT_OF_CHARACTER_PHRASES
        .iter()
        .any(|phrase| lowered.contains(phrase))
}

/// The assistant repeating one of the app's own recovery notes instead of
/// answering.
///
/// Whitespace runs are collapsed first: these are prose the model may break over
/// lines, and the phrases are quoted whole.
#[must_use]
pub fn looks_like_recovery_meta_reply(reply_text: &str) -> bool {
    let lowered = collapse_whitespace(&py_strip(reply_text).to_lowercase());
    if lowered.is_empty() {
        return false;
    }
    RECOVERY_META_PHRASES
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// A reply that hands the turn back — "Pray tell." — when the user asked
/// something that needed an answer.
///
/// A bare greeting is the one user text for which a handoff is the right reply,
/// so that pair is left alone. Only the reply's emptiness is checked: an empty
/// user text still counts as something the reply should have answered.
#[must_use]
pub fn looks_like_generic_handoff_reply(user_text: &str, reply_text: &str) -> bool {
    let user_lowered = collapse_whitespace(&py_strip(user_text).to_lowercase());
    let reply_lowered = collapse_whitespace(&py_strip(reply_text).to_lowercase());
    if reply_lowered.is_empty() {
        return false;
    }
    if searches(greeting_only(), &user_lowered) {
        return false;
    }
    HANDOFF_MARKERS
        .iter()
        .any(|marker| reply_lowered.contains(marker))
        || searches(handoff_question(), &reply_lowered)
}

/// Whether this event has already been read out.
///
/// The app polls a feed of events and the same one can arrive twice — a retry, a
/// restart, a reader that rewinds — and hearing the same news twice is the worse
/// failure. So the guard remembers the ids it has seen and answers whether an id
/// is already in that memory.
///
/// The memory is bounded, oldest-first, exactly like the `deque(maxlen=…)` the
/// source is handed: past the cap the oldest id is forgotten, which is what lets
/// a long-running poll keep working without growing without bound. A forgotten id
/// is fresh again, so a genuinely repeating event does get read out a second time
/// once enough others have passed.
///
/// Ids are JSON values rather than strings because the source compares whatever
/// it was given, and its tests feed it `None`, which is remembered and matched
/// like any other id. An id of `""` is likewise stored rather than skipped — the
/// source has no falsy short-circuit here, and inventing one would let an event
/// with no id be read out on every poll.
///
/// The source takes a lock as well, because agent.py polls from a worker thread;
/// this type owns its memory and the caller decides how to share it, which is the
/// `&mut self` rather than a lock inside.
#[derive(Debug, Default)]
pub struct RecentEvents {
    recent_ids: VecDeque<Value>,
    limit: usize,
}

impl RecentEvents {
    /// A guard that remembers up to `limit` ids, like `deque(maxlen=limit)`.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            recent_ids: VecDeque::new(),
            limit,
        }
    }

    /// Remember `event_id` and answer whether it was already remembered.
    pub fn is_duplicate(&mut self, event_id: &Value) -> bool {
        if self.recent_ids.iter().any(|seen| seen == event_id) {
            return true;
        }
        self.recent_ids.push_back(event_id.clone());
        while self.recent_ids.len() > self.limit {
            self.recent_ids.pop_front();
        }
        false
    }

    /// The ids remembered, oldest first — what the source reads back out of the
    /// deque.
    pub fn recent(&self) -> impl Iterator<Item = &Value> {
        self.recent_ids.iter()
    }
}
