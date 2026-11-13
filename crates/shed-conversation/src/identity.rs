//! Which identifier on a request names the conversation.
//!
//! Ported from `utils/conversation_history.py:132-161`. This is the first
//! production caller of `shed_store::normalize_client_id` — the reason Stage 4
//! part 1 landed before it: the id is validated once, in one place, because it is
//! text a browser wrote that ends up as a filename.

use crate::address::normalize_client_ip;
use crate::py_strip;

/// `CONVERSATION_CLIENT_PREFIX` (`conversation_history.py:132`).
pub const CONVERSATION_CLIENT_PREFIX: &str = "client-";

/// The browser id a conversation key names, or `None`.
///
/// Re-reading the id rather than trusting the string is what keeps an unvalidated
/// value from reaching a path: the remainder after the prefix goes through
/// `normalize_client_id`, and a malformed remainder is `None` rather than an
/// error, so a caller that then falls back to the address still answers the turn.
#[must_use]
pub fn conversation_client_id(conversation_key: Option<&str>) -> Option<String> {
    let text = py_strip(conversation_key.unwrap_or(""));
    let remainder = text.strip_prefix(CONVERSATION_CLIENT_PREFIX)?;
    let client = shed_store::normalize_client_id(remainder);
    if client.is_empty() {
        None
    } else {
        Some(client.to_owned())
    }
}

/// The key a conversation is stored under: the browser's id, else its address.
///
/// The id is generated once by the page and sent on every turn, so the same
/// browser always resolves to the same key — an address cannot, because every
/// computer behind one home router shares it. The server-minted replacement
/// (`shed_store::resolve_client_id`) is deliberately not used here: a client that
/// never stores it would otherwise start a new conversation on every request.
#[must_use]
pub fn conversation_identity(client_id: Option<&str>, client_ip: Option<&str>) -> Option<String> {
    let client = shed_store::normalize_client_id(client_id.unwrap_or(""));
    if !client.is_empty() {
        return Some(format!("{CONVERSATION_CLIENT_PREFIX}{client}"));
    }
    normalize_client_ip(client_ip)
}

#[cfg(test)]
mod tests {
    use super::conversation_identity;

    /// `ConversationIdentityTests` (`test_conversation_history_utils.py:104-129`).

    #[test]
    fn the_browser_id_is_preferred_over_the_address() {
        // Every computer behind one home router shares an address, so the page's
        // own id is what tells two of them apart.
        assert_eq!(
            conversation_identity(
                Some("0123456789abcdef0123456789abcdef"),
                Some("203.0.113.7")
            )
            .as_deref(),
            Some("client-0123456789abcdef0123456789abcdef")
        );
    }

    #[test]
    fn the_address_is_the_fallback_when_no_valid_id_is_sent() {
        assert_eq!(
            conversation_identity(None, Some("203.0.113.7")).as_deref(),
            Some("203.0.113.7")
        );
        // A value shaped like an id but not one (`normalize_client_id` rejects the
        // spaces and the length) is no identity either.
        assert_eq!(
            conversation_identity(Some("not a client id"), Some("203.0.113.7")).as_deref(),
            Some("203.0.113.7")
        );
    }

    #[test]
    fn the_same_request_always_names_the_same_key() {
        // The key is what determinism means here: no clock, no randomness, no
        // "newest file wins". The same browser at the same address resolves to
        // the same conversation on every turn, restart and reinstall.
        let first = conversation_identity(Some("abcdefgh12345678"), Some("203.0.113.7"));
        let second = conversation_identity(Some("abcdefgh12345678"), Some("203.0.113.7"));

        assert_eq!(first, second);
    }

    #[test]
    fn no_identifier_at_all_names_nothing() {
        assert_eq!(conversation_identity(None, None), None);
    }
}
