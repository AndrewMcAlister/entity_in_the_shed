//! The file name a conversation key is allowed to become.
//!
//! Ported from `utils/conversation_history.py:164-179` (`conversation_filename`)
//! and 181-203 (`names_a_conversation`).
//!
//! ## This is where a header value is kept out of the path
//!
//! Only a validated browser id or an address can become a name, and anything else
//! is **refused, not sanitised**. `X-Forwarded-For` and `CF-Connecting-IP` are
//! attacker-controlled text on a request that arrives from the internet, and the
//! conversation key is built from them. A sanitiser would turn `../../etc/passwd`
//! into something that still names a file; refusing it means no file is named at
//! all, which is why [`conversation_filename`] answers [`ConversationKeyError`]
//! instead of cleaning a key.
//!
//! `docs/GUARD_CATALOGUE.md` gains no row for it: the catalogue's own rule is that a
//! guard is a rule that can block, veto, cap, clamp, delay or force an *action*, and
//! reads better as a named function than as inline lines. This is a total rejection
//! of malformed input, with no state to read and no action of its own to delay or
//! force — a rule, not a gate. The guard that *acts* on the rejection is the chat
//! route's identity resolution, a later Stage-4 slice.
//!
//! ## The two spellings of one v6 address
//!
//! An IPv6 address is stored with `:` replaced by `-`, because a colon is not
//! portable through the filesystems and shells a person might meet the storage root
//! through. That makes the stored name lossy, so [`names_a_conversation`] tries both
//! spellings and accepts only the one that renders back to the same file name.

use crate::address::normalize_client_ip;
use crate::identity::{conversation_client_id, CONVERSATION_CLIENT_PREFIX};

/// A conversation key that names neither an address nor a browser.
///
/// The source raises `ValueError` with the key quoted (`{conversation_key!r}`) and
/// the `Display` keeps that wording. The quoting is Rust's `{:?}` rather than
/// Python's `repr`: both quote and both escape control characters, so neither lets a
/// hostile key forge a line of the log, and they differ only in which quote
/// character they reach for first (`docs/PARITY.md`). The key is kept rather than
/// dropped because a refusal that cannot say *which* key it refused is not
/// diagnosable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationKeyError {
    key: String,
}

impl std::fmt::Display for ConversationKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "conversation key names neither an address nor a browser: {:?}",
            self.key
        )
    }
}

impl std::error::Error for ConversationKeyError {}

/// The file name a conversation key is stored under.
///
/// The address is tried first, as the source tries it: an address is the more
/// specific statement, and a key that parses as one is never read as a browser id.
/// The dashes are applied to the *normalised* spelling, so `::ffff:192.168.1.34` and
/// `192.168.1.34` name the same file — one client reaching the app over v6 and over
/// v4 is one person, with one transcript.
///
/// # Errors
///
/// [`ConversationKeyError`] when the key is neither an address nor `client-` plus a
/// valid browser id — including the empty key, which is what a request with no
/// usable identity resolves to.
pub fn conversation_filename(conversation_key: &str) -> Result<String, ConversationKeyError> {
    if let Some(resolved) = normalize_client_ip(Some(conversation_key)) {
        return Ok(format!("{}.json", resolved.replace(':', "-")));
    }
    if let Some(client) = conversation_client_id(Some(conversation_key)) {
        return Ok(format!("{CONVERSATION_CLIENT_PREFIX}{client}.json"));
    }
    Err(ConversationKeyError {
        key: conversation_key.to_owned(),
    })
}

/// Whether a stored file name is a conversation.
///
/// Old records are named for the sha1 of a session, and the folders also hold the
/// character outline. The housekeeper must not spend a model call summarising
/// either, and retention must not treat the outline as a conversation. A `.bak` is
/// not one either: it is a copy of one, and summarising a conversation from both
/// files is how one memory gets duplicated.
#[must_use]
pub fn names_a_conversation(filename: Option<&str>) -> bool {
    let name = filename.unwrap_or("");
    let Some(stem) = name.strip_suffix(".json") else {
        return false;
    };
    if conversation_client_id(Some(stem)).is_some() {
        return true;
    }
    // An IPv6 address is stored with dashes, so both spellings are tried: the stem as
    // it is on disk, and the stem with every dash turned back into a colon. Comparing
    // the *rendered* name rather than asking whether the stem parses is what refuses
    // a stem that could be an address but would have been stored differently — a
    // bracketed `[::1]`, say, whose brackets are not part of a stored name.
    let names_this_file =
        |candidate: &str| conversation_filename(candidate).is_ok_and(|named| named == name);
    names_this_file(stem) || names_this_file(&stem.replace('-', ":"))
}

#[cfg(test)]
mod tests {
    use super::{conversation_filename, names_a_conversation};
    use crate::identity::conversation_identity;

    /// The name the source builds for a browser at an address: identity, then
    /// filename, which is the order the chat route uses.
    fn stored_name_for(client_id: &str) -> String {
        let key = conversation_identity(Some(client_id), Some("203.0.113.7")).unwrap_or_default();
        conversation_filename(&key).unwrap_or_default()
    }

    /// `ConversationFilenameTests` (`test_conversation_history_utils.py:132-168`).

    #[test]
    fn an_ipv4_address_names_the_file_itself() {
        assert_eq!(
            conversation_filename("192.168.1.34").as_deref(),
            Ok("192.168.1.34.json")
        );
    }

    #[test]
    fn an_ipv6_address_uses_dashes_because_colons_are_not_portable() {
        assert_eq!(
            conversation_filename("2001:db8::1").as_deref(),
            Ok("2001-db8--1.json")
        );
    }

    #[test]
    fn a_mapped_address_names_the_same_file_as_its_ipv4_form() {
        assert_eq!(
            conversation_filename("::ffff:192.168.1.34").as_deref(),
            Ok("192.168.1.34.json")
        );
    }

    #[test]
    fn a_browser_id_names_a_file_of_its_own() {
        assert_eq!(
            conversation_filename("client-0123456789abcdef0123456789abcdef").as_deref(),
            Ok("client-0123456789abcdef0123456789abcdef.json")
        );
    }

    #[test]
    fn two_browsers_at_one_address_name_different_files() {
        // One address, two ids: the address is identical in both calls, which is the
        // point — behind one router the id is the only thing that tells them apart.
        let first = stored_name_for("aaaaaaaaaaaaaaaa");
        let second = stored_name_for("bbbbbbbbbbbbbbbb");

        assert_eq!(first, "client-aaaaaaaaaaaaaaaa.json");
        assert_eq!(second, "client-bbbbbbbbbbbbbbbb.json");
        assert_ne!(first, second);
    }

    #[test]
    fn a_header_that_is_not_an_address_is_refused() {
        // Rejecting rather than sanitising keeps attacker-supplied text out of the
        // path entirely, and the refusal names the key it refused.
        let refused = conversation_filename("../../etc/passwd");

        assert_eq!(
            refused.err().map(|error| error.to_string()),
            Some(
                "conversation key names neither an address nor a browser: \
                 \"../../etc/passwd\""
                    .to_owned()
            )
        );
    }

    #[test]
    fn a_malformed_browser_id_is_refused() {
        // The prefix is not a licence: the remainder still has to be an id, so a
        // traversal dressed up as a browser id is refused by the validator rather
        // than by a path filter.
        assert!(conversation_filename("client-../../etc/passwd").is_err());
    }

    #[test]
    fn an_empty_key_is_refused() {
        assert!(conversation_filename("").is_err());
    }

    /// `NamesAConversationTests` (`test_conversation_history_utils.py:171-196`).

    #[test]
    fn an_ipv4_record_is_one() {
        assert!(names_a_conversation(Some("192.168.1.34.json")));
    }

    #[test]
    fn an_ipv6_record_is_one() {
        // Stored with dashes; the stem as written is not itself an address, so it is
        // the second spelling that matches.
        assert!(names_a_conversation(Some("2001-db8--1.json")));
    }

    #[test]
    fn a_browser_record_is_one() {
        assert!(names_a_conversation(Some(
            "client-0123456789abcdef0123456789abcdef.json"
        )));
    }

    #[test]
    fn a_malformed_browser_record_is_not() {
        assert!(!names_a_conversation(Some("client-.json")));
        assert!(!names_a_conversation(Some("client-not a client id.json")));
    }

    #[test]
    fn a_legacy_session_record_is_not() {
        // An address cannot be recovered from the sha1 of a session id, so old files
        // are not conversations under this rule.
        assert!(!names_a_conversation(Some(
            "conversation-562cb8b2de2c6822.json"
        )));
    }

    #[test]
    fn the_character_outline_is_not() {
        assert!(!names_a_conversation(Some("character-outline.json")));
    }

    #[test]
    fn a_backup_is_not() {
        assert!(!names_a_conversation(Some("192.168.1.34.json.bak")));
    }
}
