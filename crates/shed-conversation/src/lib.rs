//! Who a request came from, which file that is, and how much of the exact
//! transcript it may hold.
//!
//! Ported from `utils/conversation_history.py`, which is pure — no Flask, no
//! `agent` import, nothing but the stdlib and `normalize_client_id` — and this
//! crate keeps that purity: no paths opened, no settings read, no model called.
//! Everything is a function of what the caller hands it.
//!
//! ## The two rules the source's docstring states, and this slice implements
//!
//! A conversation belongs to the **browser**, identified by the id it generated
//! once and stored — `client-<id>.json` — so the same person coming back continues
//! the exchange and two computers behind one home router do not share one
//! transcript. The address is the fallback for a client that sends no id, stored as
//! itself — `192.168.1.34.json`. Neither form depends on the time or on chance: the
//! same request always names the same file.
//!
//! The exact conversation is bounded by **messages** — the bubbles the person saw —
//! because the point of the cap is the size of the prompt, and a reply stored as
//! one row per beat costs several of them.
//!
//! ## What is here, and what is waiting for the slice that needs it
//!
//! | Source | Where it goes |
//! |---|---|
//! | the four record types (42-68) | here, in `record.rs` |
//! | `_normalized_ip` (93), `resolve_client_ip` (115) | here, in `address.rs` |
//! | `conversation_client_id` (135), `conversation_identity` (148) | here, in `identity.rs` |
//! | `conversation_filename` (164), `names_a_conversation` (181) | here, in `filename.rs` |
//! | `message_window_start_index` (206) | here, in `window.rs` |
//! | `_parse_timestamp` (219), `_is_at_or_after` (228), `unprofiled_messages` (242), `newest_message_timestamp` (272), `conversation_reopened` (293) | slice 4b, in a `stamp` module: they need the permissive `datetime.fromisoformat` equivalent, which is that slice's spike |
//! | `USER_DETAILS_*` (314-321), `normalize_user_details` (340) | slice 4c, which also owns whether `user_details` keeps a `BTreeMap` or gains insertion order |
//! | `plan_conversation_compaction` (375) | slice 4c |
//! | the record paths, load/save, delivery style, history assembly (`agent.py:3785-4174`) | slice 4b — the I/O half, which is where `shed_store::atomic_write_json` gets its first caller |
//!
//! ## Why this crate is the one that pays for `shed-store`
//!
//! `normalize_client_id` (`entity_store.py:230`) landed in Stage 4 part 1 with no
//! production caller, on the strength of the caller ported here: a conversation key
//! is `client-` plus that id, and the key becomes a filename. The layer map
//! (`shed-gates`'s `LAYERS`) allows `shed-conversation → shed-store` for exactly
//! this reason.
//!
//! ## The `str(value or "")` half
//!
//! The source reads every header and body field through that falsy idiom before
//! validating it. This crate's functions take `Option<&str>` and treat `None` and
//! `""` alike, which is the same answer for the values a route can send; the
//! falsy reading of a JSON *body* field is `shed_text::text_or_empty`, and it
//! belongs at the route that reads the body (slice 4b/4d), not here. That is also
//! why `shed-text` is not a dependency yet: nothing in this slice reads a value,
//! only text a caller already stringified.

/// The lib's own test target links `serde_json` without naming it: the dev-dependency
/// is used by `tests/records.rs`, not by the unit tests here, and the workspace's
/// `unused_crate_dependencies` lint judges each target on its own uses.
#[cfg(test)]
use serde_json as _;

mod address;
mod filename;
mod identity;
mod record;
mod window;

pub use address::{normalize_client_ip, resolve_client_ip};
pub use filename::{conversation_filename, names_a_conversation, ConversationKeyError};
pub use identity::{conversation_client_id, conversation_identity, CONVERSATION_CLIENT_PREFIX};
pub use record::{
    ConversationCompactionPlan, ConversationMessage, ConversationRecord, ConversationSummary,
};
pub use window::message_window_start_index;

/// Python's `str.strip()` with no argument.
///
/// The gap worth naming, and the same one `shed-store` names for its copy: Python
/// treats `\u{1c}`–`\u{1f}` as whitespace and [`char::is_whitespace`] does not, so a
/// header value starting with one of those would otherwise be refused here and
/// accepted there.
///
/// Two crates now carry this helper because each keeps its own private copy:
/// `shed-store` validates an id, this crate validates an address, and neither may
/// depend on the other's internals (`shed-store`'s copy is private, and the layer
/// map has no reason to expose it). A third need for it moves it to `shed-text`,
/// which both are allowed to reach.
pub(crate) fn py_strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}
