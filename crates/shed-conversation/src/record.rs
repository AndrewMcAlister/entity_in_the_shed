//! The records a conversation is stored as.
//!
//! Ported from `utils/conversation_history.py:42-68`, where they are `TypedDict`
//! and `NamedTuple` declarations. They are the shape of the JSON on disk, so the
//! one thing that can be got silently wrong here is written back wrong: the three
//! fields the source marks `NotRequired` must be **absent** when a record was
//! written before they existed, not `null`. `#[serde(default,
//! skip_serializing_if = "Option::is_none")]` on each is what keeps a round-trip
//! of an old file byte-for-byte old, and `tests/records.rs` pins it.
//!
//! ## What each type is for
//!
//! | Type | Source | Role |
//! |---|---|---|
//! | [`ConversationMessage`] | 42 | one row of the exact transcript: what was said, and when |
//! | [`ConversationSummary`] | 50 | one block of remembered material, covering a span already trimmed |
//! | [`ConversationRecord`] | 56 | a conversation as stored: the exact tail and the memory behind it |
//! | [`ConversationCompactionPlan`] | 67 | what one housekeeping pass should do with a record |
//!
//! ## The fields that arrived late, and why that matters
//!
//! `delivery_style` is how the last turn was spoken: the page forgets its style on
//! every reload, so continuing a conversation means remembering it.
//! `user_details` is the flat topic-to-phrase profile of the person, and
//! `user_details_covered_through` names the newest message that profile has read —
//! a timestamp rather than a count, because trimming removes rows from the front.
//! All three are optional because records written before they existed carry none,
//! and those files are still the person's memory.
//!
//! ## The decision this file used to leave open, and the one it still does
//!
//! `user_details` is [`UserDetails`] rather than a `BTreeMap`, which is the
//! decision `SLICE-4a-BRIEF` deferred to slice 4c: Python's dict keeps the order
//! the model wrote its topics in, that order is what the file stores and what the
//! rendered profile shows, and a map would silently sort it — so every rewrite of
//! an old record would reorder the person's own facts. The type, its two impls and
//! the function that fills it are `profile.rs`; what this file decides is only
//! which field of the record that profile is, and that it is optional.
//!
//! What is still not decided here is tolerance rather than shape: `messages` and
//! `summaries` are required, as the `TypedDict` declares them, and tolerating a
//! stored file that carries neither is the loader's job — the loader is slice 4d
//! (`agent.py:3785-4174`), and a row that is not an object fails when the record is
//! parsed rather than being skipped per loop (`docs/PARITY.md`).

use serde::{Deserialize, Serialize};

use crate::profile::UserDetails;

/// One row of the exact transcript: what was said, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    /// `"user"` or `"assistant"`, stored as the source writes it.
    pub role: String,
    /// What was said. A reply stored as one row per beat costs several of these.
    pub content: String,
    /// The moment the row was stored, in the source's own ISO-8601 spelling.
    /// Read, never re-rendered — see `newest_message_timestamp` (slice 4b).
    pub timestamp: String,
}

/// One block of remembered material, covering a span already trimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// The first row this block remembers.
    pub start_timestamp: String,
    /// The last row this block remembers.
    pub end_timestamp: String,
    /// What was kept of the span, in the model's own words.
    pub summary: String,
}

/// A conversation as stored: the exact tail and the memory behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRecord {
    /// The exact transcript, oldest row first. Trimming takes rows from the front.
    pub messages: Vec<ConversationMessage>,
    /// The remembered spans behind the exact tail, oldest block first.
    pub summaries: Vec<ConversationSummary>,
    /// The style the last turn was delivered in. Absent on a record written before
    /// the field existed, and on one whose first turn has not ended yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_style: Option<String>,
    /// The profile of the person: one short phrase per topic, in the order they
    /// were written. Absent until the first idle pass fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_details: Option<UserDetails>,
    /// The newest message the profile has read. A timestamp, not a count, because
    /// trimming removes rows from the front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_details_covered_through: Option<String>,
}

/// What one housekeeping pass should do with a record.
///
/// `consumed_count` names the prefix of the stored messages this plan accounts
/// for. It is carried rather than recomputed so a turn that lands while the
/// summary is being written is not dropped along with it: the housekeeper removes
/// exactly the rows the plan read.
///
/// The source spells this a `NamedTuple`; a named struct is the same thing with
/// the field names on the outside, which is what the caller needs when it removes
/// "exactly the rows the plan read". It is never stored, so it is not `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationCompactionPlan {
    /// The rows to fold into a summary before they leave the exact transcript.
    pub messages_to_summarize: Vec<ConversationMessage>,
    /// The rows the record keeps.
    pub retained_messages: Vec<ConversationMessage>,
    /// The summary blocks the record keeps.
    pub retained_summaries: Vec<ConversationSummary>,
    /// How many stored rows this plan accounted for, including any the age cut
    /// dropped without summarising.
    pub consumed_count: usize,
}
