//! The `reply` string, out of a JSON document that has not finished arriving.
//!
//! Port of `JsonReplyStreamScanner` from `agent.py:5592-5799`.
//!
//! ## It never guesses
//!
//! [`crate::parse_model_json_content`] is the authority on what a reply may look
//! like, but it needs the whole document. This scanner recognises only the
//! layout the prompt actually asks for — an object whose first key is `reply`
//! (or `response`) and whose value is a string — and gives up on anything else.
//! A layout it cannot follow would otherwise speak half a sentence that the final
//! parse then contradicts, and a sentence cannot be unsaid, so those turns fall
//! back to the whole-document behaviour instead.
//!
//! ## Why the machine is this exact
//!
//! Every state is a character the model has already committed to: text before
//! the first `{` is a preamble it has to be given a bounded chance to finish
//! writing ([`JsonReplyStreamScanner::MAX_PREAMBLE_CHARS`]), a key is only worth
//! following while some known key still *starts with* the letters so far, and a
//! reply value that turns out not to be a string is abandoned rather than
//! rendered. Inside the string, the only held-back text is a `\uXXXX` escape and
//! the high half of a surrogate pair, which is why the publisher may speak
//! everything else the moment it arrives.
//!
//! ## Lone surrogates
//!
//! Rust's `char` cannot hold a surrogate, where Python's `str` holds a code unit
//! happily and only fails at encode time. Both answers here are the one Python
//! reaches for in the same situation: the replacement character, written out at
//! the point the pair is known never to complete — which is *before* the
//! character that proved it, so the reply never reads with the character
//! misplaced.

use crate::py_whitespace_char;

/// `_JSON_STRING_ESCAPES`.
const JSON_STRING_ESCAPES: [(char, char); 8] = [
    ('"', '"'),
    ('\\', '\\'),
    ('/', '/'),
    ('b', '\u{0008}'),
    ('f', '\u{000C}'),
    ('n', '\n'),
    ('r', '\r'),
    ('t', '\t'),
];

/// The character a code point that cannot be encoded becomes.
const REPLACEMENT: char = '\u{fffd}';

/// The states of the machine, in the order the document walks through them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    /// Nothing seen yet — the first non-space character decides everything.
    #[default]
    Start,
    /// Inside a triple-backtick fence, looking for the `{` that starts the
    /// document.
    Fence,
    /// Inside the object, before its first key.
    Object,
    /// Reading a key.
    Key,
    /// Between the key and its value.
    Colon,
    /// Reading the value.
    Value,
    /// Inside the reply string.
    String,
    /// The reply string closed. Terminal.
    Done,
    /// The document is not streamable. Terminal.
    Aborted,
}

/// Pull the `reply` string out of a JSON reply while it is still arriving.
#[derive(Clone, Debug, Default)]
pub struct JsonReplyStreamScanner {
    state: State,
    preamble_chars: usize,
    key_text: String,
    reply: String,
    aborted: bool,
    /// A backslash has been read and its escape character has not.
    pending_escape: bool,
    /// The digits of a `\uXXXX` escape read so far; `Some("")` right after the
    /// `u`, which is why this is an option over a string rather than a count.
    unicode_digits: Option<String>,
    /// The high half of a surrogate pair, waiting for its low half.
    pending_high_surrogate: Option<u32>,
}

impl JsonReplyStreamScanner {
    /// The keys worth streaming — the object's first key has to be one of them.
    pub const KEYS: [&'static str; 2] = ["reply", "response"];

    /// How much text is allowed before the document opens. A prose preamble is
    /// not streamable, so the hunt is bounded rather than grown without limit by
    /// a model that never opens an object.
    pub const MAX_PREAMBLE_CHARS: usize = 256;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The reply text recognised so far.
    #[must_use]
    pub fn reply(&self) -> &str {
        &self.reply
    }

    /// The document was abandoned; nothing more will ever be emitted.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.aborted
    }

    /// The reply string closed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Consume a chunk and return the reply text it completed.
    ///
    /// A missing chunk is the empty string, which is what `str(delta or "")`
    /// means at the call site in the agent loop.
    pub fn feed(&mut self, delta: Option<&str>) -> String {
        let mut emitted = String::new();
        for character in delta.unwrap_or_default().chars() {
            emitted.push_str(&self.feed_char(character));
        }
        emitted
    }

    /// Close the stream, returning text still held back for lookahead.
    pub fn finish(&mut self) -> String {
        self.flush_surrogate()
    }

    /// One character of the document, answered with the reply text it completed.
    fn feed_char(&mut self, character: char) -> String {
        if matches!(self.state, State::Done | State::Aborted) {
            return String::new();
        }

        match self.state {
            State::Start => self.start(character),
            State::Fence => self.fence(character),
            State::Object => self.object(character),
            State::Key => self.key(character),
            State::Colon => self.colon(character),
            State::Value => self.value(character),
            State::String => self.string(character),
            State::Done | State::Aborted => String::new(),
        }
    }

    fn start(&mut self, character: char) -> String {
        if py_whitespace_char(character) {
            return String::new();
        }
        // The preamble count is not returned here: a preamble that overflows is
        // an abort, and an abort flushes a surrogate that cannot be pending in
        // this state. The `fence` state below does return it, and does so for
        // the same reason it is harmless.
        let _ignored = self.count_preamble();
        if character == '`' {
            self.state = State::Fence;
            return String::new();
        }
        if character == '{' {
            self.state = State::Object;
            return String::new();
        }
        self.abort()
    }

    fn fence(&mut self, character: char) -> String {
        // ``` or ```json — the tag is optional and never contains a brace.
        if character == '{' {
            self.state = State::Object;
            return String::new();
        }
        // Whitespace belongs here too: the tag is followed by the newline that
        // separates it from the document, and a fence that aborted on its own
        // line break would fall back on every fenced reply the model writes.
        if character == '`'
            || character == '-'
            || character == '_'
            || character.is_alphanumeric()
            || py_whitespace_char(character)
        {
            return self.count_preamble();
        }
        self.abort()
    }

    fn object(&mut self, character: char) -> String {
        if py_whitespace_char(character) {
            return String::new();
        }
        if character == '"' {
            self.key_text.clear();
            self.state = State::Key;
            return String::new();
        }
        self.abort()
    }

    fn key(&mut self, character: char) -> String {
        if character == '"' {
            if !Self::KEYS.contains(&self.key_text.as_str()) {
                return self.abort();
            }
            self.state = State::Colon;
            return String::new();
        }
        if character.is_alphabetic() || character == '_' {
            self.key_text.push(character);
            if !Self::KEYS
                .iter()
                .any(|key| key.starts_with(self.key_text.as_str()))
            {
                return self.abort();
            }
            return String::new();
        }
        self.abort()
    }

    fn colon(&mut self, character: char) -> String {
        if py_whitespace_char(character) {
            return String::new();
        }
        if character == ':' {
            self.state = State::Value;
            return String::new();
        }
        self.abort()
    }

    fn value(&mut self, character: char) -> String {
        if py_whitespace_char(character) {
            return String::new();
        }
        if character == '"' {
            self.state = State::String;
            return String::new();
        }
        // A non-string reply is not worth streaming, and guessing at it would
        // only produce text the final parse disagrees with.
        self.abort()
    }

    fn string(&mut self, character: char) -> String {
        if self.pending_escape {
            self.pending_escape = false;
            return self.feed_escape(character);
        }
        // Still collecting the four digits of a \uXXXX escape.
        if self.unicode_digits.is_some() {
            return self.feed_escape(character);
        }
        if character == '\\' {
            self.pending_escape = true;
            return String::new();
        }
        if character == '"' {
            self.state = State::Done;
            return self.flush_surrogate();
        }
        // Only another \uXXXX escape can complete a held high surrogate, so
        // ordinary text means it never will be — and it has to be written out
        // here, not at the end, or the reply reads with the character misplaced.
        if self.pending_high_surrogate.is_some() {
            let flushed = self.flush_surrogate();
            self.reply.push(character);
            return flushed + &character.to_string();
        }
        self.reply.push(character);
        character.to_string()
    }

    fn count_preamble(&mut self) -> String {
        self.preamble_chars += 1;
        if self.preamble_chars > Self::MAX_PREAMBLE_CHARS {
            return self.abort();
        }
        String::new()
    }

    fn feed_escape(&mut self, character: char) -> String {
        // Taking the digits out is what both exits below want: the malformed one
        // has already decided they were not an escape, the collecting one puts
        // them straight back.
        if let Some(digits) = self.unicode_digits.take() {
            if character.is_ascii_hexdigit() {
                let collected = format!("{digits}{character}");
                if collected.chars().count() < 4 {
                    self.unicode_digits = Some(collected);
                    return String::new();
                }
                // Every character of `collected` is a hex digit by construction.
                let code = u32::from_str_radix(&collected, 16).unwrap_or_default();
                return self.emit_code_point(code);
            }
            // Malformed \u escape. Keep what was written and carry on with this
            // character rather than losing the rest of the reply.
            let literal = format!("\\u{digits}");
            self.reply.push_str(&literal);
            return literal + &self.feed_char(character);
        }

        if character == 'u' {
            self.unicode_digits = Some(String::new());
            return String::new();
        }

        // Any other escape forecloses on a held high surrogate.
        let flushed = self.flush_surrogate();
        // JSON forbids escapes like \q. Dropping the backslash loses less than
        // aborting a reply the model is halfway through.
        let mapped = JSON_STRING_ESCAPES
            .iter()
            .find(|(from, _)| *from == character)
            .map_or(character, |(_, to)| *to);
        self.reply.push(mapped);
        flushed + &mapped.to_string()
    }

    fn emit_code_point(&mut self, code: u32) -> String {
        let high = self.pending_high_surrogate;
        if (0xD800..=0xDBFF).contains(&code) {
            self.pending_high_surrogate = Some(code);
            if high.is_some() {
                self.reply.push(REPLACEMENT);
                return REPLACEMENT.to_string();
            }
            return String::new();
        }
        if (0xDC00..=0xDFFF).contains(&code) {
            self.pending_high_surrogate = None;
            let Some(high) = high else {
                self.reply.push(REPLACEMENT);
                return REPLACEMENT.to_string();
            };
            let paired = char::from_u32(0x10000 + ((high - 0xD800) << 10) + (code - 0xDC00))
                .unwrap_or(REPLACEMENT);
            self.reply.push(paired);
            return paired.to_string();
        }
        self.pending_high_surrogate = None;
        // `chr(code)` for a code point past the Unicode range is a `ValueError`
        // in Python; the replacement character is the answer that keeps a reply
        // arriving.
        let mut resolved = char::from_u32(code).unwrap_or(REPLACEMENT).to_string();
        if high.is_some() {
            resolved.insert(0, REPLACEMENT);
        }
        self.reply.push_str(&resolved);
        resolved
    }

    fn abort(&mut self) -> String {
        self.aborted = true;
        self.state = State::Aborted;
        self.flush_surrogate()
    }

    fn flush_surrogate(&mut self) -> String {
        // A high surrogate with no partner cannot be encoded, so it becomes the
        // usual replacement character rather than an unencodable lone code unit
        // that would break the very JSON response this text is reported in.
        if self.pending_high_surrogate.is_none() {
            return String::new();
        }
        self.pending_high_surrogate = None;
        self.reply.push(REPLACEMENT);
        REPLACEMENT.to_string()
    }
}
