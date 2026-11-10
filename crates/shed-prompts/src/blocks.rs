//! Named prompt blocks: the names this app gives them, the placeholders it
//! sends, and the request fields that name them.
//!
//! All of `utils/prompt_cache.py` lives here, plus the name check of
//! `character_prompt_cache_id` (`agent.py:3062`). Nothing here talks HTTP and
//! nothing here decides whether the store is there — that is the stage that
//! asks the engine. The module is the names, the placeholders and the payload
//! fields, so all of it can be tested without one, exactly as the Python
//! docstring says.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use fancy_regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use shed_text::text_or_empty;

/// `CHARACTER_BLOCK_PREFIX` (`utils/prompt_cache.py:23`).
pub const CHARACTER_BLOCK_PREFIX: &str = "character_";

/// `USER_INFO_BLOCK_PREFIX` (`utils/prompt_cache.py:24`).
pub const USER_INFO_BLOCK_PREFIX: &str = "user_info_";

/// `USER_INFO_BLOCK_ID_LENGTH` (`utils/prompt_cache.py:35`): how much of the
/// digest a conversation block's name carries.
pub const USER_INFO_BLOCK_ID_LENGTH: usize = 20;

/// The characters Strata accepts in a block name, and its length limit
/// (`utils/prompt_cache.py:26-29`): 1-200 of letters, digits and `. _ : + -`.
fn is_block_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '+' | '-')
}

/// `is_prompt_block_name` (`utils/prompt_cache.py:38`): whether Strata will
/// accept this as a block's name.
///
/// Equivalent to `PROMPT_BLOCK_NAME.fullmatch(name)`, spelled as a length and
/// character check rather than a pattern: `{1,200}` counts *characters*, which
/// a byte-length check would get wrong for any non-ASCII name. The Python
/// `isinstance(name, str)` half is in the signature — a non-string value is
/// not a name, and a caller holding a JSON value reaches it through `as_str`.
#[must_use]
pub fn is_prompt_block_name(name: &str) -> bool {
    (1..=200).contains(&name.chars().count()) && name.chars().all(is_block_name_char)
}

/// The one compiled `PROMPT_BLOCK_REFERENCE` (`utils/prompt_cache.py:30`).
///
/// `\A`/`\z` stand in for the anchors `fullmatch` implies; the pattern is
/// substituted through, so it is `fancy_regex`, whose `$` would otherwise
/// match before a trailing newline the way Python's does.
fn block_reference() -> Option<&'static Regex> {
    static REFERENCE: OnceLock<Option<Regex>> = OnceLock::new();
    REFERENCE
        .get_or_init(|| Regex::new(r"\{\{cache:([A-Za-z0-9._:+-]{1,200})\}\}").ok())
        .as_ref()
}

/// `character_block_key` (`utils/prompt_cache.py:43`): the block holding one
/// character's card, shared by everyone who picks it.
#[must_use]
pub fn character_block_key(character_cache_id: &str) -> String {
    format!("{CHARACTER_BLOCK_PREFIX}{}", character_cache_id.trim())
}

/// `user_info_block_key` (`utils/prompt_cache.py:48`): the block holding one
/// conversation's user details and summary.
///
/// Empty without a conversation: there is nothing to name. The character is
/// part of the name because a conversation's record lives in the character's
/// folder, so the same browser talking to two characters has two records.
/// The NUL between the two halves is the separator that keeps
/// `("a", "b")` from colliding with `("ab", "")` — no key or id can hold it.
#[must_use]
pub fn user_info_block_key(conversation_key: &str, character_identity: &str) -> String {
    let conversation = conversation_key.trim();
    if conversation.is_empty() {
        return String::new();
    }
    let identity = character_identity.trim();
    let digest = Sha256::digest(format!("{conversation}\u{0}{identity}").as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{USER_INFO_BLOCK_PREFIX}{}",
        &hex[..USER_INFO_BLOCK_ID_LENGTH]
    )
}

/// `prompt_block_placeholder` (`utils/prompt_cache.py:63`): the text a message
/// carries in place of a block's value.
#[must_use]
pub fn prompt_block_placeholder(key: &str) -> String {
    // `"{{cache:%s}}" % …`: the `%` operator takes braces literally, so the
    // placeholder is double-braced — which is what the reference regex and
    // Strata's expander both look for. `str(key or "")` reads an empty key
    // as the empty name, and the trim is the source's `.strip()`.
    format!("{{{{cache:{}}}}}", key.trim())
}

/// `is_prompt_block_listing` (`utils/prompt_cache.py:68`): whether a
/// `GET /cache` body is a Strata block listing.
///
/// A server that happens to answer 200 on an unknown route is not a block
/// store; the listing's own shape — an object whose `entries` is a list — is
/// what tells them apart.
#[must_use]
pub fn is_prompt_block_listing(payload: &Value) -> bool {
    payload.get("entries").is_some_and(Value::is_array)
}

/// `body_names_missing_prompt_block` (`utils/prompt_cache.py:78`): whether a
/// refused request's body says it could not expand a block.
///
/// Only these refusals are worth retrying inline; any other 400 is the
/// request's own fault and would fail the same way a second time.
#[must_use]
pub fn body_names_missing_prompt_block(body: &str) -> bool {
    let text = body.to_lowercase();
    text.contains("cache block") || text.contains("cache_pin")
}

/// `expand_prompt_blocks_text` (`utils/prompt_cache.py:90`): `text` with every
/// named block replaced by its value.
///
/// A name the mapping does not hold is left as written. Every placeholder this
/// app sends was recorded in the same turn it was sent, so a miss here means a
/// caller built the prompt by hand; leaving the name visible is better than
/// dropping it silently.
#[must_use]
pub fn expand_prompt_blocks_text(text: &str, blocks: &BTreeMap<String, String>) -> String {
    if !text.contains("{{cache:") || blocks.is_empty() {
        return text.to_owned();
    }
    let Some(re) = block_reference() else {
        // A pattern that fails to compile expands nothing; it does not.
        return text.to_owned();
    };
    re.replace_all(text, |caps: &fancy_regex::Captures<'_, str>| {
        let name = caps.get(1).map(|name| name.as_str());
        match name.and_then(|name| blocks.get(name)) {
            Some(value) => value.clone(),
            // `match.group(0)`: the placeholder as written.
            None => caps
                .get(0)
                .map_or_else(String::new, |m| m.as_str().to_owned()),
        }
    })
    .into_owned()
}

/// `expand_prompt_blocks` (`utils/prompt_cache.py:108`): a copy of `messages`
/// with every placeholder in a string content expanded.
///
/// Used only on the inline fallback, when the engine refused the blocks. The
/// input is never edited: the caller may retry the same prompt again, and the
/// placeholders are what the store was asked for.
#[must_use]
pub fn expand_prompt_blocks(messages: &Value, blocks: &BTreeMap<String, String>) -> Value {
    let items: Vec<Value> = match messages {
        Value::Array(items) => items.clone(),
        // `messages or []`: anything else falsy iterates empty. A non-list
        // that is not falsy would raise `TypeError` in Python; no caller
        // sends one.
        _ => Vec::new(),
    };
    Value::Array(
        items
            .into_iter()
            .map(|mut message| {
                let expanded = message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| expand_prompt_blocks_text(content, blocks));
                if let (Some(object), Some(text)) = (message.as_object_mut(), expanded) {
                    object.insert("content".to_owned(), Value::String(text));
                }
                message
            })
            .collect(),
    )
}

/// `PromptCachePlan` (`utils/prompt_cache.py:123`): one turn's blocks, and the
/// request fields that name them.
///
/// The plan collects the text of every block the prompt refers to, because the
/// caller has to store those blocks before the engine is asked to expand them.
/// [`PromptCachePlan::reference`] is what puts a placeholder in the prompt, so
/// a plan can never name a block whose text it does not hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptCachePlan {
    /// The engine's client id for this conversation.
    pub client: String,
    /// The block that ends the shared start of the prompt.
    pub pin: String,
    /// The text of every block the prompt refers to.
    pub blocks: BTreeMap<String, String>,
}

impl PromptCachePlan {
    /// `PromptCachePlan(client=…, pin=…)` with no blocks yet.
    #[must_use]
    pub fn new(client: &str, pin: &str) -> Self {
        Self {
            client: client.to_owned(),
            pin: pin.to_owned(),
            blocks: BTreeMap::new(),
        }
    }

    /// `PromptCachePlan.reference` (`utils/prompt_cache.py:137`): hold `text`
    /// under `key` for this turn and return its placeholder.
    pub fn reference(&mut self, key: &str, text: &str) -> String {
        self.blocks.insert(key.to_owned(), text.to_owned());
        prompt_block_placeholder(key)
    }

    /// `PromptCachePlan.request_fields` (`utils/prompt_cache.py:142`): the
    /// chat request's cache fields, omitting anything not named.
    ///
    /// Pinning a block the prompt does not refer to is refused, so `cache_pin`
    /// is sent only when this plan holds it.
    #[must_use]
    pub fn request_fields(&self) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        if !self.pin.is_empty() && self.blocks.contains_key(&self.pin) {
            fields.insert("cache_pin".to_owned(), self.pin.clone());
        }
        if !self.client.is_empty() {
            fields.insert("cache_client".to_owned(), self.client.clone());
        }
        fields
    }
}

/// The name check of `character_prompt_cache_id` (`agent.py:3062`): the
/// prompt-cache id of a character, the country file's if Strata will take it.
///
/// The authored id is what makes one character's block reusable by every user
/// who picks it. A file written before ids existed still gets one, derived
/// from the same identity, so the block's name is stable either way — and
/// minting that derived id belongs to `shed-characters`, which this crate may
/// not reach into, so the caller hands the derivation in as `derived_id`.
#[must_use]
pub fn character_prompt_cache_id(
    character_profile: &Value,
    derived_id: impl Fn(&Value) -> String,
) -> String {
    let profile = match character_profile {
        Value::Object(_) => character_profile,
        // `character_profile if isinstance(…, dict) else {}`.
        _ => &Value::Null,
    };
    let authored = text_or_empty(profile.get("cache_id")).trim().to_owned();
    if is_prompt_block_name(&authored) {
        return authored;
    }
    derived_id(profile)
}
