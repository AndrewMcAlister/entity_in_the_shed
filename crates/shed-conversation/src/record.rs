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
//! ## Two things this file deliberately does not decide
//!
//! `user_details` is a `BTreeMap` for now: Python dicts keep insertion order and
//! the prompt may render the facts in it, so **slice 4c owns that decision** and
//! may replace the map (SLICE-4a-BRIEF decision 1). Nothing here orders or renders
//! the facts in the meantime.
//!
//! `messages` and `summaries` are required, as the `TypedDict` declares them.
//! Tolerating a stored file that carries neither is the loader's job, and the
//! loader is slice 4b (`agent.py:3785-4174`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    /// The profile of the person: short topic-to-phrase facts. Absent until the
    /// first idle pass fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_details: Option<BTreeMap<String, String>>,
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
