//! The reply document — what one engine turn's text becomes before anyone reads it.
//!
//! Port of the reply-document half of `agent.py:6037-6202`
//! (`REPLY_DOCUMENT_KEYS`, `_parse_structured_candidate`,
//! `_parse_model_reply_document`, `reply_without_prose_tool_calls`,
//! `settle_prose_tool_calls`, `parse_model_json_content`) and of the
//! module-level patterns those use, all of which the rest of `agent.py` calls
//! rather than re-implementing.
//!
//! ## `ReplyDocument` is a JSON object, not a struct
//!
//! The brief draws it as `{ reply: String, tool_calls: Vec<Value> }`. The
//! fixtures are the reason it is not: `_parse_model_reply_document` returns
//! *whatever object the model named*, so `{"reply": "ok", "extra_key": 1}` has
//! to come back with `extra_key` still on it, `{'reply': None, 'tool_calls':
//! None}` has to keep both nulls (a `setdefault` only fills a key that is
//! absent), `{"replies": […]}` has to keep its `replies` list, and
//! [`settle_prose_tool_calls`] has to be able to add and remove
//! `reply_segments`. A struct with two fields answers none of those. So it is a
//! newtype over a `serde_json` object with the keys the source names as
//! accessors, and [`parse_structured_candidate`] is the one place that decides
//! whether a parse produced a document at all.
//!
//! ## The Python-literal parser
//!
//! `_parse_structured_candidate` tries `json.loads` and then `ast.literal_eval`.
//! The second is a full Python-expression evaluator; what the fixtures and the
//! model actually put through it is a dict written with single quotes, so this
//! is a hand-written parser for exactly that grammar — single- **and**
//! double-quoted strings, `None`/`True`/`False`, nested dicts, lists, tuples and
//! trailing commas — and nothing else. Anything it cannot follow answers `None`
//! and the caller falls through to the regex fallbacks, which is the same answer
//! `literal_eval` gives when it raises.
//!
//! ## `str(value or "")`
//!
//! The source coerces the reply with `str(parsed.get("reply") or "")`, so a
//! document whose reply is the number `42` reads `"42"` and one whose reply is
//! `false` reads `""`. [`text_or_empty`] is that expression over a `Value`.

use fancy_regex::Regex;
use serde_json::{Map, Value};
use shed_text::{py_repr, py_str, text_or_empty};

use crate::quality::{extract_reply_segments, tool_calls_in_envelope};
use crate::reply_text::normalize_reply_str;
use crate::strip_tool_call_envelope;

/// The keys that make a parsed object a reply document rather than some other
/// JSON the model decided to return.
const REPLY_DOCUMENT_KEYS: [&str; 4] = ["reply", "replies", "response", "tool_calls"];

// `re.sub(r"```(?:json)?\s*|\s*```$", "", text, flags=re.IGNORECASE | re.DOTALL)`
// — the fences off a markdown-wrapped document. An alternation, so this is one
// `replace_all` rather than two, and the first branch is deliberately
// unanchored: Python removes a fence from anywhere in the text.
pattern!(fenced_json, r"(?is)```(?:json)?\s*|\s*```$");

// `re.search(r"\\{.*\\}", text, flags=re.DOTALL)` — greedy, so from the first
// `{` to the *last* `}`, which is what makes a document wrapped in prose
// recoverable.
pattern!(json_candidate, r"(?s)\{.*\}");

// `re.search(r'"reply"\s*:\s*"((?:\\.|[^"\\])*)"', text, flags=re.DOTALL)`
pattern!(reply_value, r#""reply"\s*:\s*"((?:\\.|[^"\\])*)""#);

// `re.search(r'"reply"\s*:\s*"(?P<reply>.*)$', text, flags=re.DOTALL)` — the
// truncated-document fallback: a reply key whose string never closed.
pattern!(partial_reply_value, r#""reply"\s*:\s*"(.*)$"#);

// `re.sub(r'"\s*,?\s*"tool_calls"\s*:\s*\[.*$', "", …, flags=re.DOTALL)`
pattern!(
    partial_tool_calls_tail,
    r#""\s*,?\s*"tool_calls"\s*:\s*\[.*$"#
);

// `re.sub(r'"\\s*}*\\s*$', "", …)`
pattern!(partial_quote_tail, r#""\s*}*\s*$"#);

/// A parsed reply document: a JSON object with the keys the source names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplyDocument(Value);

impl ReplyDocument {
    /// The document with an empty reply and no tool calls — what
    /// [`parse_model_json_content`] answers for text that yields no document at
    /// all, which is `{"reply": "", "tool_calls": []}` rather than nothing.
    #[must_use]
    pub fn empty() -> Self {
        let mut object = Map::new();
        object.insert("reply".to_owned(), Value::String(String::new()));
        object.insert("tool_calls".to_owned(), Value::Array(Vec::new()));
        Self(Value::Object(object))
    }

    /// The value behind a key, if the document has it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// The reply as text — `str(x or "")` over the stored value, which is what
    /// every reader in the source does with it.
    #[must_use]
    pub fn reply(&self) -> String {
        text_or_empty(self.get("reply"))
    }

    /// The tool calls, or no calls at all when the key is absent or null.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<Value> {
        self.get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// The stored `reply_segments`, one entry per bubble.
    ///
    /// This reads the key rather than re-deriving bubbles from `replies`: the
    /// parse body writes it only when a turn has more than one beat, so its
    /// *presence* is the signal the client reads, and re-deriving it here would
    /// answer with the `replies` list of a document that has since settled down
    /// to a single bubble — which [`settle_prose_tool_calls`] removes.
    /// Each entry is read with `str(x or "")`, as every reader in the source does.
    #[must_use]
    pub fn reply_segments(&self) -> Vec<String> {
        self.get("reply_segments")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|segment| text_or_empty(Some(segment)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The document as JSON, for the server to serialize.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// Take the JSON out of it.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.0
    }

    /// Wrap a parsed object, keeping every key the model named.
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        Self(value)
    }

    /// `document["reply"] = text`.
    fn set_reply(&mut self, text: &str) {
        if let Value::Object(object) = &mut self.0 {
            object.insert("reply".to_owned(), Value::String(text.to_owned()));
        }
    }

    /// `document["tool_calls"] = calls`.
    fn set_tool_calls(&mut self, calls: &[Value]) {
        if let Value::Object(object) = &mut self.0 {
            object.insert("tool_calls".to_owned(), Value::Array(calls.to_vec()));
        }
    }

    /// `"key" in document`, which a stored `null` answers true.
    fn has_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// `document.pop("reply_segments", None)`.
    fn remove_key(&mut self, key: &str) {
        if let Value::Object(object) = &mut self.0 {
            object.remove(key);
        }
    }

    /// `document["reply_segments"] = segments`.
    fn set_reply_segments(&mut self, segments: &[String]) {
        if let Value::Object(object) = &mut self.0 {
            object.insert(
                "reply_segments".to_owned(),
                Value::Array(
                    segments
                        .iter()
                        .map(|text| Value::String(text.clone()))
                        .collect(),
                ),
            );
        }
    }

    /// `document.setdefault("tool_calls", [])`, which leaves a stored `null`
    /// exactly as null.
    fn default_tool_calls(&mut self) {
        if let Value::Object(object) = &mut self.0 {
            object
                .entry("tool_calls".to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
        }
    }
}

/// `re.search(pattern, text)` answered with the matched text — the whole match,
/// which is what both candidate searches use (`match.group(0)`).
fn first_match<'t>(pattern: Option<&Regex>, text: &'t str) -> Option<&'t str> {
    let found = pattern.and_then(|regex| regex.find(text).ok().flatten())?;
    text.get(found.range())
}

/// `re.search(pattern, text).group(n)` — the engine gave up, or there was no
/// match, or the group did not participate, all three of which read as "no
/// candidate" to the callers below.
fn first_capture<'t>(pattern: Option<&Regex>, text: &'t str, group: usize) -> Option<&'t str> {
    let captures = pattern.and_then(|regex| regex.captures(text).ok().flatten())?;
    captures
        .get(group)
        .and_then(|found| text.get(found.range()))
}

/// `re.sub(pattern, replacement, text)`, answering `text` unchanged when the
/// pattern is absent or the engine gave up. `NoExpand` because every
/// replacement here is a literal; Python's `re.sub` would expand `\1`, but none
/// of these replacements hold a backslash, so the two agree.
fn replace_all(pattern: Option<&Regex>, text: &str, replacement: &str) -> String {
    pattern
        .and_then(|regex| {
            regex
                .try_replacen(text, 0, fancy_regex::NoExpand(replacement))
                .ok()
        })
        .map_or_else(|| text.to_owned(), |replaced| replaced.into_owned())
}

/// Pull a reply document out of text that is meant to *be* one: JSON first,
/// then a Python literal, then nothing.
pub fn parse_structured_candidate(text: &str) -> Option<ReplyDocument> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return document_from_parsed(value);
    }
    python_literal(text).and_then(document_from_parsed)
}

/// The `parsed = …` guard of `_parse_model_reply_document`: a JSON object, or a
/// Python literal dict, and only when it names one of the reply-document keys.
fn document_from_parsed(value: Value) -> Option<ReplyDocument> {
    let named = value.as_object().is_some_and(|object| {
        REPLY_DOCUMENT_KEYS
            .iter()
            .any(|key| object.contains_key(*key))
    });
    if named {
        Some(ReplyDocument(value))
    } else {
        None
    }
}

/// Turn one engine turn's text into a reply document.
///
/// Three candidates in the source's order — the text itself, the same with its
/// markdown fences removed (only for text that *starts* with a fence, since a
/// fence in the middle of prose is prose), and the slice from the first `{` to
/// the last `}` (only when there is one) — then two regex fallbacks for
/// documents that never parsed: one that reads a complete `"reply"` value out of
/// the text, and one that reads a reply the engine stopped halfway through.
/// Failing all of those the reply is the text itself, which is the behaviour
/// every turn had before reply documents existed.
///
/// A candidate that parses is settled by [`settle_reply_fields`] and returned
/// *without* the envelope pass: the source keeps that to
/// [`parse_model_json_content`], the one entry point every parse goes through,
/// so the regex fallbacks below are settled by it too.
#[must_use]
pub fn parse_model_reply_document(text: &str) -> ReplyDocument {
    let text = text.trim();
    if text.is_empty() {
        return ReplyDocument::empty();
    }

    let mut candidates: Vec<String> = vec![text.to_owned()];
    if text.starts_with("```") {
        let fenced = crate::py_strip(&replace_all(fenced_json(), text, "")).to_owned();
        if !fenced.is_empty() {
            candidates.push(fenced);
        }
    }
    if let Some(braced) = first_match(json_candidate(), text) {
        candidates.push(braced.to_owned());
    }

    for candidate in &candidates {
        if let Some(mut document) = parse_structured_candidate(candidate) {
            settle_reply_fields(&mut document);
            return document;
        }
    }

    if let Some(encoded) = first_capture(reply_value(), text, 1) {
        // `json.loads(f'"{encoded}"')`, which is the JSON string grammar over
        // the captured body; a value that will not decode is used raw, exactly
        // as the source's bare `except Exception` answers.
        let decoded = serde_json::from_str::<String>(&format!("\"{encoded}\""))
            .unwrap_or_else(|_| encoded.to_owned());
        return ReplyDocument::from_value(document_pair(&normalize_reply_str(&decoded)));
    }

    if let Some(partial) = first_capture(partial_reply_value(), text, 1) {
        let without_calls = replace_all(partial_tool_calls_tail(), partial, "");
        let without_quote = replace_all(partial_quote_tail(), &without_calls, "");
        // The source's five `.replace` calls, in this order: `\\n` before `\\`
        // means a literal backslash-n in the reply is already a newline by the
        // time the backslash rule runs.
        let reply = without_calls_to_text(&without_quote);
        return ReplyDocument::from_value(document_pair(&normalize_reply_str(&reply)));
    }

    ReplyDocument::from_value(document_pair(&normalize_reply_str(text)))
}

/// Fill in the reply fields of a parsed candidate: the `response` alias, then
/// the `replies` bubbles, then `tool_calls` defaulted.
///
/// The order is the source's and it is observable. A `response` string only
/// becomes the reply when there is no `reply` **key** at all — a stored `null`
/// under `reply` is a key, so it wins and the reply comes back empty. And
/// `reply_segments` is set only when there is more than one bubble, so its
/// presence is itself the signal that the turn wants more than one bubble; a
/// single `replies` entry merely fills in a missing reply, because otherwise a
/// stray one-element array would silently override an explicit `reply`.
fn settle_reply_fields(document: &mut ReplyDocument) {
    if !document.has_key("reply") {
        if let Some(Value::String(response)) = document.get("response") {
            let response = response.clone();
            document.set_reply(&response);
        }
    }

    let bubbles: Vec<String> =
        extract_reply_segments(document.get("replies").unwrap_or(&Value::Null))
            .iter()
            .map(|segment| normalize_reply_str(segment))
            .filter(|segment| !segment.is_empty())
            .collect();

    if bubbles.len() > 1 {
        document.set_reply_segments(&bubbles);
        document.set_reply(&normalize_reply_str(&bubbles.join("\n\n")));
    } else {
        // `str(parsed.get("reply") or "").strip() or (segments[0] if segments else "")`
        let stored = document.reply();
        let explicit = crate::py_strip(&stored);
        let fallback = if explicit.is_empty() {
            bubbles.first().cloned().unwrap_or_default()
        } else {
            explicit.to_owned()
        };
        document.set_reply(&normalize_reply_str(&fallback));
    }

    document.default_tool_calls();
}

/// The two `.replace` lines of the partial-reply fallback, verbatim and in order:
///
/// ```text
/// partial_reply.replace('\\r\\n', '\n').replace('\\n', '\n').replace('\\r', '\n')
/// partial_reply.replace('\\"', '"').replace('\\\\', '\\')
/// ```
///
/// Each Python literal there is *escaped*, so the text being looked for is a
/// literal backslash followed by a letter — the two-character shape `\n` a model
/// writes when it cannot emit a newline. `\\n` is handled before `\\`, so a
/// literal backslash-n is already a newline by the time the backslash rule runs,
/// and that is why `\\` sits last rather than first.
fn without_calls_to_text(text: &str) -> String {
    text.replace(r"\r\n", "\n")
        .replace(r"\n", "\n")
        .replace(r"\r", "\n")
        .replace("\\\"", "\"")
        .replace(r"\\", "\\")
}

/// `{"reply": …, "tool_calls": []}` — the shape both regex fallbacks return.
fn document_pair(reply: &str) -> Value {
    let mut object = Map::new();
    object.insert("reply".to_owned(), Value::String(reply.to_owned()));
    object.insert("tool_calls".to_owned(), Value::Array(Vec::new()));
    Value::Object(object)
}

/// Text with any prose tool-call envelope removed — byte-identical when there is none.
///
/// Renormalising only when something was removed is what the source's docstring
/// asks for: it keeps a reply that never had an envelope *exactly* as it was,
/// rather than passing it through the normaliser a second time on its way to the
/// client, which is where a normaliser's opinions would quietly start landing in
/// text that had already been settled once.
#[must_use]
pub fn reply_without_prose_tool_calls(reply_text: &str) -> String {
    let settled = strip_tool_call_envelope(reply_text);
    if settled == reply_text {
        reply_text.to_owned()
    } else {
        normalize_reply_str(&settled)
    }
}

/// Move any prose tool-call envelope out of the reply and into `tool_calls`.
///
/// The reply is settled first, then every bubble, and the reply is rebuilt from
/// the settled bubbles when more than one survives — because `reply` is defined
/// by the parse body as their join, and stripping it in place would leave a hole
/// where the envelope was that disagrees with the bubbles the client shows. A
/// document that drops down to one bubble has `reply_segments` removed rather
/// than left as a one-element list: its presence is the signal for more than one
/// bubble, so a bubble that was nothing but an envelope leaves the turn
/// single-bubble instead of adding an empty one to speak aloud.
///
/// The Python warns when the text changed, naming the tools inside the envelope
/// and quoting the reply it found them in; that is the `tracing::warn!` here.
/// `%s` over a list is its `repr`, and `%r` over `reply[:300]` is a *character*
/// slice, so both are spelled through [`py_repr`] rather than through Rust's
/// `{:?}`, which quotes with `"` and escapes differently.
pub fn settle_prose_tool_calls(document: &mut ReplyDocument) {
    let reply = document.reply();
    let recovered = tool_calls_in_envelope(&reply);
    let settled = reply_without_prose_tool_calls(&reply);

    if settled != reply {
        // A marker reached the reply, which by construction means the model wrote
        // the envelope as prose and the call was about to be lost. Never silent again.
        let names: Vec<Value> = recovered
            .iter()
            .map(|call| call.get("tool").cloned().unwrap_or(Value::Null))
            .collect();
        let quoted_tools = py_repr(&Value::Array(names));
        let quoted_reply = py_repr(&Value::String(reply.chars().take(300).collect()));
        tracing::warn!(
            "Prose tool-call envelope in the reply: tools={quoted_tools} text={quoted_reply}"
        );
        document.set_reply(&settled);
    }

    // The key is only ever written by the parse body, as a non-empty array, and
    // an empty one is falsy and takes the same `pop` as an absent one. A value of
    // any other type is what Python would raise a `TypeError` over; here it reads
    // as no bubbles and so also takes the `pop`.
    if truthy(document.get("reply_segments")) {
        let bubbles: Vec<String> = document
            .reply_segments()
            .iter()
            .map(|segment| reply_without_prose_tool_calls(segment))
            .filter(|segment| !segment.is_empty())
            .collect();
        if bubbles.len() > 1 {
            document.set_reply_segments(&bubbles);
            document.set_reply(&normalize_reply_str(&bubbles.join("\n\n")));
        } else {
            document.remove_key("reply_segments");
        }
    }

    if !recovered.is_empty() && !truthy(document.get("tool_calls")) {
        document.set_tool_calls(&recovered);
    }
}

/// The whole of `parse_model_json_content`: parse, then settle.
///
/// The envelope pass belongs to this entry point and to nothing else, which is
/// what makes it unconditional: every parse — a structured document, a reply
/// read out of a truncated one, or plain prose — goes through it on its way to
/// storage, the client, and the speech path.
#[must_use]
pub fn parse_model_json_content(text: &str) -> ReplyDocument {
    let mut document = parse_model_reply_document(text);
    settle_prose_tool_calls(&mut document);
    document
}

/// Python truthiness over a JSON value — the `bool(x)` an `or` chain asks.
/// Mirrors the private copy in `shed-characters`; this crate may not depend on
/// it, and a NaN number is truthy and `-0.0` falsy exactly as Python's are.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_none_or(|value| value != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// The Python-literal grammar
// ---------------------------------------------------------------------------

/// `ast.literal_eval` over the subset a model emits: dicts, lists, tuples,
/// strings in either quote style, numbers, `None`/`True`/`False`.
///
/// A `{` whose entries are bare expressions — a Python set — becomes an array,
/// because `literal_eval` accepts it and the caller only rejects the result for
/// not being a dict.
fn python_literal(text: &str) -> Option<Value> {
    let mut parser = Literal {
        chars: text.chars().collect(),
        at: 0,
    };
    parser.skip_space();
    let value = parser.value()?;
    parser.skip_space();
    // `literal_eval` rejects trailing text; so does this.
    if parser.at != parser.chars.len() {
        return None;
    }
    Some(value)
}

struct Literal {
    chars: Vec<char>,
    at: usize,
}

impl Literal {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(crate::py_whitespace_char) {
            self.at += 1;
        }
    }

    fn starts_with(&self, word: &str) -> bool {
        let width = word.chars().count();
        self.chars.len() >= self.at + width
            && self.chars[self.at..self.at + width]
                .iter()
                .copied()
                .eq(word.chars())
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            '{' => self.mapping(),
            '[' => self.sequence(']'),
            '(' => self.sequence(')'),
            '"' | '\'' => self.string().map(Value::String),
            _ => self.atom(),
        }
    }

    /// `{'a': 1, 'b': 2, }` — and also `{'a'}`, which is a Python set. The set
    /// comes back as an array so the caller can reject it for not being a dict,
    /// exactly as `literal_eval`'s `set` is rejected for not being a `dict`.
    fn mapping(&mut self) -> Option<Value> {
        self.at += 1;
        let mut object = Map::new();
        // A `{` whose entries turn out to have no `:` is a set; keep those in
        // order rather than as keys, which a `Map` would sort.
        let mut members: Vec<Value> = Vec::new();
        loop {
            self.skip_space();
            if self.eat('}') {
                break;
            }
            let key = self.value()?;
            self.skip_space();
            if self.eat(':') {
                self.skip_space();
                let value = self.value()?;
                object.insert(key_name(&key)?, value);
            } else {
                members.push(key);
            }
            self.skip_space();
            if self.eat('}') {
                break;
            }
            if !self.eat_comma() {
                return None;
            }
        }
        if members.is_empty() {
            return Some(Value::Object(object));
        }
        members.extend(object.into_values());
        Some(Value::Array(members))
    }

    fn eat(&mut self, ch: char) -> bool {
        if self.peek() == Some(ch) {
            self.at += 1;
            return true;
        }
        false
    }

    /// `,` between entries, tolerated before the closer — Python allows a
    /// trailing comma in every display.
    fn eat_comma(&mut self) -> bool {
        self.eat(',')
    }

    fn sequence(&mut self, closer: char) -> Option<Value> {
        self.at += 1;
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if self.peek() == Some(closer) {
                self.at += 1;
                break;
            }
            items.push(self.value()?);
            self.skip_space();
            if self.peek() == Some(closer) {
                self.at += 1;
                break;
            }
            if !self.eat_comma() {
                return None;
            }
        }
        Some(Value::Array(items))
    }

    /// Python's string grammar, which is the JSON one plus `'` quotes. The
    /// escapes a reply document ever contains are the JSON ones.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.at += 1;
        let mut text = String::new();
        loop {
            let ch = self.peek()?;
            self.at += 1;
            if ch == quote {
                return Some(text);
            }
            if ch != '\\' {
                text.push(ch);
                continue;
            }
            let escaped = self.peek()?;
            self.at += 1;
            match escaped {
                'n' => text.push('\n'),
                't' => text.push('\t'),
                'r' => text.push('\r'),
                'b' => text.push('\u{0008}'),
                'f' => text.push('\u{000C}'),
                '0' => text.push('\0'),
                '\\' => text.push('\\'),
                '"' => text.push('"'),
                '\'' => text.push('\''),
                'u' | 'U' => {
                    let width = if escaped == 'u' { 4 } else { 8 };
                    let mut code = 0u32;
                    for _ in 0..width {
                        let digit = self.peek()?.to_digit(16)?;
                        self.at += 1;
                        code = code * 16 + digit;
                    }
                    text.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                }
                other => {
                    text.push('\\');
                    text.push(other);
                }
            }
        }
    }

    fn atom(&mut self) -> Option<Value> {
        for (word, value) in [
            ("None", Value::Null),
            ("True", Value::Bool(true)),
            ("False", Value::Bool(false)),
        ] {
            if self.starts_with(word) {
                self.at += word.len();
                return Some(value);
            }
        }
        let start = self.at;
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.' | '_') {
                text.push(ch);
                self.at += 1;
            } else {
                break;
            }
        }
        if self.at == start {
            return None;
        }
        // A bare word is not a literal: `literal_eval` raises, and so does this.
        text.parse::<i64>().map_or_else(
            |_| {
                text.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
            },
            |number| Some(Value::from(number)),
        )
    }
}

/// The key of a dict entry as JSON wants it. `literal_eval` accepts keys of any
/// literal type and the source's readers compare them as `str()`, so a `None`
/// key is the text `"None"` here exactly as it is there.
fn key_name(key: &Value) -> Option<String> {
    match key {
        Value::String(text) => Some(text.clone()),
        Value::Null => Some("None".to_owned()),
        Value::Bool(true) => Some("True".to_owned()),
        Value::Bool(false) => Some("False".to_owned()),
        other => Some(py_str(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_plain_json_object_is_a_document() {
        let document = parse_structured_candidate(r#"{"reply": "json"}"#);
        assert_eq!(document.map(|doc| doc.reply()), Some("json".to_owned()));
    }

    #[test]
    fn an_object_without_a_reply_key_is_not() {
        assert!(parse_structured_candidate(r#"{"other": 1}"#).is_none());
        assert!(parse_structured_candidate("[1, 2]").is_none());
        assert!(parse_structured_candidate("").is_none());
    }

    #[test]
    fn a_single_quoted_literal_is_a_document() {
        let document = parse_structured_candidate("{'reply': 'literal', 'tool_calls': [],}")
            .unwrap_or_else(|| panic!("a single-quoted literal should parse"));
        assert_eq!(document.reply(), "literal");
        assert!(document.tool_calls().is_empty());
    }

    #[test]
    fn nulls_survive_the_parse() {
        let document = parse_structured_candidate("{'reply': None, 'tool_calls': None}")
            .unwrap_or_else(|| panic!("`None` is JSON `null`, not a syntax error"));
        assert_eq!(document.reply(), "");
        assert_eq!(document.get("tool_calls"), Some(&Value::Null));
    }

    #[test]
    fn a_tuple_inside_a_literal_is_an_array() {
        let document = parse_structured_candidate("{'reply': 'r', 'tool_calls': [(1, 2)]}")
            .unwrap_or_else(|| panic!("a tuple is a literal the way a list is"));
        assert_eq!(document.get("tool_calls"), Some(&json!([[1, 2]])));
    }

    #[test]
    fn a_reply_document_defaults_to_an_empty_reply_and_no_calls() {
        let value = ReplyDocument::empty().into_value();
        let object = value
            .as_object()
            .unwrap_or_else(|| panic!("an empty document is an object"));
        assert_eq!(object.get("reply"), Some(&Value::String(String::new())));
        assert_eq!(object.get("tool_calls"), Some(&Value::Array(Vec::new())));
    }
}
