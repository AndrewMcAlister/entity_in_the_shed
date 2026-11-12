//! The client id: the string a browser generated once and stored, which the
//! server turns into a filename.
//!
//! Ported from `utils/entity_store.py:230-248`. It is validated here and nowhere
//! else, because it is untrusted input that ends up in a path: the page sends it,
//! and a request from the internet can send anything. `client_file_path` (Stage 7)
//! is the second line of defence, hashing the id before it names a file.

/// `CLIENT_ID_PATTERN` (`entity_store.py:30`): `^[A-Za-z0-9_-]{8,64}$`.
///
/// The check is written out rather than matched, because the pattern is an
/// anchored ASCII class with a counted repeat, which is two conditions on the
/// same bytes: every byte in the alphabet, and the count inside the range. It is
/// equivalent for every input, including the ones it rejects — a non-ASCII
/// character fails the class, so a string whose byte length differs from its
/// character length can never be the one where counting bytes instead of
/// characters would matter.
fn is_client_id(text: &str) -> bool {
    (8..=64).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Python's `str.strip()` with no argument.
///
/// The gap worth naming: Python treats `\u{1c}`–`\u{1f}` as whitespace and Rust's
/// [`char::is_whitespace`] does not, so a value starting with one of those would
/// otherwise be rejected here and accepted there.
fn strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// The validated client id, or `""` when it is missing or malformed.
///
/// The caller hands over the text it read off the request — the source spells
/// `str(raw or "")`, which is `shed_text::text_or_empty` at the call site (see
/// the crate doc for why the layer map keeps that reading outside this crate).
///
/// The empty string is the answer meaning "no usable identity": it is what
/// `conversation_identity` (`utils/conversation_history.py:158`) falls through to
/// the address on, and what `resolve_client_id` mints over.
#[must_use]
pub fn normalize_client_id(raw: &str) -> &str {
    let candidate = strip(raw);
    if is_client_id(candidate) {
        candidate
    } else {
        ""
    }
}

/// A fresh identity: `uuid4().hex`, thirty-two lower-case hex characters.
#[must_use]
pub fn mint_client_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// A validated client id, or a fresh one.
///
/// Never fails. A browser that sends nothing — an older cached page, a cleared
/// `localStorage` — gets a server-minted identity and keeps working, where
/// rejecting the request would break the chat entirely to protect a feature it
/// was not using.
#[must_use]
pub fn resolve_client_id(raw: &str) -> String {
    let normalized = normalize_client_id(raw);
    if normalized.is_empty() {
        mint_client_id()
    } else {
        normalized.to_owned()
    }
}
