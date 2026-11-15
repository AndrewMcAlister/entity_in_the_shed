//! The conversation as the model should see it: profile, memory, exact tail.
//!
//! Ported from `agent.py:4082-4174` — `build_user_details_system_text`,
//! `build_summary_system_text` and `get_conversation_history`, which here is
//! [`conversation_history`]: the same reading, named for what it returns rather
//! than for the key that names a file.
//!
//! ## Why the order is the shape
//!
//! The profile comes first because it is the most durable thing the conversation
//! knows, and a person who has been away is greeted by the name it holds. Memory is
//! the gist of older talk, and the exact tail is last, because the newest words are
//! the ones the reply has to answer. That is the source's own ordering and its own
//! reason, and it is the one thing here a later refactor could silently break: two
//! system messages swapped is a profile the model reads after it has already read
//! the words the profile explains.
//!
//! ## Reading, not loading
//!
//! The source loads the record inside its own body. This takes the
//! [`ConversationRecord`] the caller already holds, which is what makes the order
//! testable without a temporary directory — and it is why there is no second read of
//! a file another request may be appending to. Trimming stays the idle housekeeper's
//! work (`plan_conversation_compaction`, 4c): a turn must not spend the request
//! summarizing older material. The message cap, though, *is* applied here, so the
//! prompt never carries more rows than the stored conversation keeps.
//!
//! ## The prompt-block seam is `shed_prompts::blocks`'
//!
//! With a [`PromptCachePlan`], the profile text and the summary text become one
//! named block for this conversation and the history carries its placeholder in one
//! system message instead of two. Without one they are the two messages they have
//! always been. The block is *named* here and *stored* by the caller before the
//! engine is asked to expand it, exactly as in the source, so this crate never talks
//! to the engine and the two modes are one `match`.
//!
//! ## Values, not globals
//!
//! `CONVERSATION_MAX_MESSAGES` and `CONVERSATION_REUNION_SECONDS` are environment
//! reads in the source; no crate in this workspace reads the environment outside
//! `shed-config`, so both arrive as parameters. So does `now`, which the source takes
//! from `datetime.now().astimezone()` — a local clock, where every cutoff this app
//! otherwise uses is `datetime.now(timezone.utc)`. The clone passes UTC and the
//! stamps it compares were written by that same UTC clock; the difference is recorded
//! in `docs/PARITY.md`, because a local offset is hours of reunion gap for anyone not
//! on UTC.

use std::time::Duration;

use chrono::{DateTime, Utc};
use shed_prompts::blocks::{user_info_block_key, PromptCachePlan};

use crate::freshness::conversation_reopened;
use crate::profile::{normalize_user_details, UserDetails, USER_DETAILS_OTHER_LABEL};
use crate::py_strip;
use crate::record::{ConversationMessage, ConversationRecord, ConversationSummary};
use crate::timestamp::Timestamp;
use crate::window::message_window_start_index;

/// The largest quiet threshold a [`Duration`] can hold.
///
/// `Duration::from_secs` refuses a count it cannot represent, and the refusal is a
/// panic — so the boundary is applied here rather than discovered by a config value
/// nobody read twice. Anything above it is a gap no transcript reaches, which is the
/// same answer the boundary itself gives.
const MAX_QUIET_SECONDS: u64 = u64::MAX / 1_000_000_000;

/// One message as the model is asked to see it: a role and its text.
///
/// Not [`ConversationMessage`]: a stored row carries the moment it was written, and
/// the prompt does not want it. The source builds these as two-key dicts inside a
/// comprehension, which is the same pair of fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryMessage {
    /// `"system"`, `"user"` or `"assistant"`, as stored or as the two texts say.
    pub role: String,
    /// What the model reads: a rendered text, or a row exactly as it was spoken.
    pub content: String,
}

/// What this conversation knows about the person, as one system message's text.
///
/// Ported from `build_user_details_system_text` (4082-4109). Empty when nothing is
/// known — no profile, or one with no usable phrase left in it — because an empty
/// system message is a line in the prompt that says nothing.
///
/// The reunion line is added only when a name is known **and** the transcript shows
/// a gap: the conversation outlives the visit, so the gap in the transcript can say
/// what the page cannot. Either half alone is not a return, and the two tests that
/// pin that are worth more than the sentence explaining it.
///
/// The stored profile goes back through [`normalize_user_details`], as the source's
/// first line does, so a hand-edited record whose topic has an empty phrase renders
/// as the absence the source renders rather than as `- name:`.
///
/// `reunion_seconds` is the source's `CONVERSATION_REUNION_SECONDS`; anything at or
/// below zero disables the greeting, as its `<= 0` does.
#[must_use]
pub fn build_user_details_system_text(
    details: Option<&UserDetails>,
    messages: &[ConversationMessage],
    now: DateTime<Utc>,
    reunion_seconds: i64,
) -> String {
    let Some(stored) = details else {
        return String::new();
    };
    let profile = renormalize(stored);
    if profile.is_empty() {
        return String::new();
    }

    // `other` last, everything else alphabetical — the source's sort key, and stable
    // either way. The underscores become spaces on the way out, not in the stored
    // profile: `family` is the key the file and the model agree on, `family` read as
    // two words is what the prompt shows.
    let mut ordered: Vec<(&str, &str)> = profile
        .pairs()
        .iter()
        .map(|(label, value)| (label.as_str(), value.as_str()))
        .collect();
    ordered.sort_by_key(|&(label, _)| (label == USER_DETAILS_OTHER_LABEL, label));
    let mut lines: Vec<String> = ordered
        .iter()
        .map(|(label, value)| format!("- {}: {}", label.replace('_', " "), value))
        .collect();

    let name = py_strip(profile.get("name").unwrap_or("")).to_owned();
    if !name.is_empty()
        && conversation_reopened(messages, utc_stamp(now), quiet_gap(reunion_seconds))
    {
        lines.push(format!(
            "They have just come back after a break. Greet them by their name ({name}) before answering."
        ));
    }

    format!(
        "What this conversation knows about the person:\n{}",
        lines.join("\n")
    )
}

/// The gist of older talk, as one system message's text; empty when none.
///
/// Ported from `build_summary_system_text` (4112-4121). A summary whose text is empty
/// or all whitespace contributes no line — the source's `str(... or "").strip()` test
/// — and a block of nothing is not worth a message.
///
/// The line carries the text **as stored**, padding and all, which is what the
/// source's f-string does: the strip decides whether a row counts, not what is
/// written. The typed rows make the source's `isinstance(item, dict)` disappear
/// (`docs/PARITY.md`, 4b).
#[must_use]
pub fn build_summary_system_text(summaries: &[ConversationSummary]) -> String {
    let lines: Vec<String> = summaries
        .iter()
        .filter(|item| !py_strip(&item.summary).is_empty())
        .map(|item| format!("- {}", item.summary))
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("Conversation summary memory:\n{}", lines.join("\n"))
}

/// The conversation as the model should see it: profile, memory, exact tail.
///
/// Ported from `get_conversation_history` (4124-4174), with the load taken out
/// because the caller already holds the record — see the module docs. Read-only in
/// every sense: nothing here trims, summarizes or rewrites a file.
///
/// No key, no history. That is the source's first line, and it is also what keeps an
/// unnamed conversation from naming a block:
/// [`user_info_block_key`] is empty without a conversation, and a placeholder for an
/// empty name is a block the engine cannot find.
///
/// The tail is the stored rows from [`message_window_start_index`] on, in stored
/// order, with their roles and contents as stored — no re-rendering, no trimming of
/// content. A cap of zero keeps none of them, leaving the two memory messages alone.
///
/// `character_identity` is the caller's `character_prompt_cache_identity` equivalent,
/// and `""` when there is no character; it is part of the block's name because one
/// browser talking to two characters has two records.
#[must_use]
pub fn conversation_history(
    record: &ConversationRecord,
    conversation_key: &str,
    character_identity: &str,
    max_messages: i64,
    now: DateTime<Utc>,
    reunion_seconds: i64,
    prompt_cache: Option<&mut PromptCachePlan>,
) -> Vec<HistoryMessage> {
    if conversation_key.is_empty() {
        return Vec::new();
    }

    let exact_start = message_window_start_index(record.messages.len(), max_messages);
    let user_details_text = build_user_details_system_text(
        record.user_details.as_ref(),
        &record.messages,
        now,
        reunion_seconds,
    );
    let summary_text = build_summary_system_text(&record.summaries);

    let mut history: Vec<HistoryMessage> = Vec::new();
    if let Some(cache) = prompt_cache {
        // One block for both texts, and one message carrying its placeholder. The
        // caller stores what the plan holds before the engine expands it.
        let user_info_text = join_blocks(&[user_details_text, summary_text]);
        if !user_info_text.is_empty() {
            let key = user_info_block_key(conversation_key, character_identity);
            history.push(system_message(cache.reference(&key, &user_info_text)));
        }
    } else {
        for text in [user_details_text, summary_text] {
            if !text.is_empty() {
                history.push(system_message(text));
            }
        }
    }

    history.extend(
        record.messages[exact_start..]
            .iter()
            .map(|item| HistoryMessage {
                role: item.role.clone(),
                content: item.content.clone(),
            }),
    );
    history
}

/// A system message, which is what both memory texts and a placeholder are.
fn system_message(content: String) -> HistoryMessage {
    HistoryMessage {
        role: "system".to_owned(),
        content,
    }
}

/// The non-empty parts joined by a blank line, as the source's
/// `"\n\n".join(part for part in … if part)` joins them.
fn join_blocks(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// The stored profile, put back through 4c's [`normalize_user_details`] the way the
/// source's first line puts its raw dict through it.
///
/// On a profile this crate wrote the call has nothing to remove — it is the shape the
/// writer stored — so it only ever fires on a file that was edited by hand or written
/// by an older app. `UserDetails` is the normalized shape rather than a `Value`, so
/// the trip goes through a `Value` and back; rendering a list of two-string pairs
/// cannot fail, and were it ever to, the stored profile is the only reading of itself
/// left.
fn renormalize(details: &UserDetails) -> UserDetails {
    match serde_json::to_value(details) {
        Ok(value) => normalize_user_details(&value),
        Err(_) => details.clone(),
    }
}

/// `now` on the timeline the stored stamps live on.
///
/// A UTC instant is an aware one at zero offset, which is what every stamp this app
/// has written carries. Read the other way — a naive cutoff — the pair would order
/// nowhere and no turn would ever be a return (`timestamp.rs` explains that pair).
fn utc_stamp(now: DateTime<Utc>) -> Timestamp {
    Timestamp::Aware(now.fixed_offset())
}

/// The quiet gap, as a [`Duration`] that cannot panic and cannot lie.
fn quiet_gap(reunion_seconds: i64) -> Duration {
    match u64::try_from(reunion_seconds) {
        // The source's `quiet_seconds <= 0` disables the greeting, and
        // `conversation_reopened` reads zero the same way.
        Err(_) => Duration::ZERO,
        Ok(seconds) if seconds > MAX_QUIET_SECONDS => Duration::MAX,
        Ok(seconds) => Duration::from_secs(seconds),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{
        build_summary_system_text, build_user_details_system_text, conversation_history,
        HistoryMessage,
    };
    use crate::profile::UserDetails;
    use crate::record::{ConversationMessage, ConversationRecord, ConversationSummary};
    use shed_prompts::blocks::PromptCachePlan;

    /// The moment the housekeeping suite fixes rather than reads off a clock, so a
    /// test run at midnight behaves as one at noon.
    fn utc(text: &str) -> DateTime<Utc> {
        match DateTime::parse_from_rfc3339(text) {
            Ok(stamp) => stamp.to_utc(),
            Err(error) => panic!("{text} should parse as an instant: {error}"),
        }
    }

    const NOON: &str = "2026-10-27T12:00:00+00:00";
    const REUNION_SECONDS: i64 = 3600;
    const MAX_MESSAGES: i64 = 10;

    fn message(role: &str, content: &str, timestamp: &str) -> ConversationMessage {
        ConversationMessage {
            role: role.to_owned(),
            content: content.to_owned(),
            timestamp: timestamp.to_owned(),
        }
    }

    /// A row written an hour ago: long enough ago to be a return, and the only
    /// fixture the reunion line needs beside `NOON`.
    fn older(content: &str) -> ConversationMessage {
        message("user", content, "2026-10-27T10:00:00+00:00")
    }

    fn profile(pairs: &[(&str, &str)]) -> UserDetails {
        UserDetails::new(
            pairs
                .iter()
                .map(|(label, value)| ((*label).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn nothing_known_is_no_text() {
        assert_eq!(
            build_user_details_system_text(None, &[older("hello")], utc(NOON), REUNION_SECONDS),
            ""
        );
        // An empty profile is the same answer: an empty system message would say
        // "what this conversation knows" and then nothing.
        assert_eq!(
            build_user_details_system_text(
                Some(&profile(&[])),
                &[older("hello")],
                utc(NOON),
                REUNION_SECONDS
            ),
            ""
        );
    }

    #[test]
    fn the_profile_is_one_system_texts_worth_of_lines() {
        // Listed name-first, rendered likes-first: the sort key puts the topics in
        // alphabetical order rather than leaving them in the order the file stored
        // them, so a profile that grew over months reads the same whatever order the
        // model happened to notice things in.
        let text = build_user_details_system_text(
            Some(&profile(&[("name", "Andrew"), ("likes", "woodwork")])),
            &[],
            utc(NOON),
            REUNION_SECONDS,
        );

        assert_eq!(
            text,
            "What this conversation knows about the person:\n- likes: woodwork\n- name: Andrew"
        );
    }

    #[test]
    fn other_is_the_last_line_however_it_was_stored() {
        // The sort key is `(label == "other", label)`: `other` last, everything else
        // alphabetical. Stored order is what the file keeps, and what the prompt does
        // not show.
        let text = build_user_details_system_text(
            Some(&profile(&[
                ("other", "he hums while he works"),
                ("workshop", "the shed"),
                ("age", "41"),
            ])),
            &[],
            utc(NOON),
            REUNION_SECONDS,
        );

        assert_eq!(
            text,
            "What this conversation knows about the person:\n- age: 41\n- workshop: the shed\n- other: he hums while he works"
        );
    }

    #[test]
    fn a_stored_topics_underscores_are_read_as_spaces() {
        let text = build_user_details_system_text(
            Some(&profile(&[("sick_mother", "a sick mum")])),
            &[],
            utc(NOON),
            REUNION_SECONDS,
        );

        assert!(
            text.contains("- sick mother: a sick mum"),
            "rendered as: {text}"
        );
    }

    #[test]
    fn a_topic_with_no_phrase_is_no_line_at_all() {
        // The source normalizes before it renders, so an empty phrase is dropped
        // rather than shown as a key with nothing after the colon.
        let text = build_user_details_system_text(
            Some(&profile(&[("name", "Andrew"), ("hopes", "   ")])),
            &[],
            utc(NOON),
            REUNION_SECONDS,
        );

        assert_eq!(
            text,
            "What this conversation knows about the person:\n- name: Andrew"
        );
    }

    #[test]
    fn the_reunion_line_needs_a_name_and_a_gap() {
        // Both halves, in both orders: a name with no gap is a person still standing
        // there, and a gap with no name is nobody to greet.
        let named = profile(&[("name", "Andrew")]);
        let recently = [message(
            "user",
            "one more thing",
            "2026-10-27T11:59:00+00:00",
        )];

        let alone = build_user_details_system_text(Some(&named), &[], utc(NOON), REUNION_SECONDS);
        assert!(
            !alone.contains("come back after a break"),
            "no transcript is no gap: {alone}"
        );

        let recent =
            build_user_details_system_text(Some(&named), &recently, utc(NOON), REUNION_SECONDS);
        assert!(
            !recent.contains("come back after a break"),
            "a minute is not a break: {recent}"
        );

        let unnamed = build_user_details_system_text(
            Some(&profile(&[("workshop", "the shed")])),
            &[older("hello")],
            utc(NOON),
            REUNION_SECONDS,
        );
        assert!(
            !unnamed.contains("come back after a break"),
            "there is nobody to greet: {unnamed}"
        );

        let back = build_user_details_system_text(
            Some(&named),
            &[older("hello")],
            utc(NOON),
            REUNION_SECONDS,
        );
        assert!(
            back.ends_with("- name: Andrew\nThey have just come back after a break. Greet them by their name (Andrew) before answering."),
            "the line belongs at the end: {back}"
        );
    }

    #[test]
    fn a_zero_gap_disables_the_greeting() {
        let text = build_user_details_system_text(
            Some(&profile(&[("name", "Andrew")])),
            &[older("hello")],
            utc(NOON),
            0,
        );

        assert!(!text.contains("come back after a break"), "{text}");
        // A negative setting from a config read is the same disabling, not a gap of
        // nothing that every transcript clears.
        let negative = build_user_details_system_text(
            Some(&profile(&[("name", "Andrew")])),
            &[older("hello")],
            utc(NOON),
            -1,
        );
        assert!(!negative.contains("come back after a break"), "{negative}");
    }

    #[test]
    fn remembered_talk_is_one_line_per_block() {
        let summaries = [
            ConversationSummary {
                start_timestamp: "2026-10-20T09:00:00+00:00".to_owned(),
                end_timestamp: "2026-10-26T18:00:00+00:00".to_owned(),
                summary: "We talked about the shed.".to_owned(),
            },
            ConversationSummary {
                start_timestamp: "2026-10-26T18:00:00+00:00".to_owned(),
                end_timestamp: "2026-10-27T09:00:00+00:00".to_owned(),
                summary: "   ".to_owned(),
            },
        ];

        assert_eq!(
            build_summary_system_text(&summaries),
            "Conversation summary memory:\n- We talked about the shed."
        );
        assert_eq!(build_summary_system_text(&summaries[1..]), "");
        assert_eq!(build_summary_system_text(&[]), "");
    }

    /// A record with a profile, one remembered block and three exact rows.
    fn record() -> ConversationRecord {
        ConversationRecord {
            messages: vec![
                message("user", "first", "2026-10-27T11:50:00+00:00"),
                message("assistant", "second", "2026-10-27T11:52:00+00:00"),
                message("user", "third", "2026-10-27T11:58:00+00:00"),
            ],
            summaries: vec![ConversationSummary {
                start_timestamp: "2026-10-20T09:00:00+00:00".to_owned(),
                end_timestamp: "2026-10-26T18:00:00+00:00".to_owned(),
                summary: "We talked about the shed.".to_owned(),
            }],
            delivery_style: Some("warm".to_owned()),
            user_details: Some(profile(&[("name", "Andrew")])),
            user_details_covered_through: None,
        }
    }

    fn row(role: &str, content: &str) -> HistoryMessage {
        HistoryMessage {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn the_history_is_profile_then_memory_then_the_exact_tail() {
        let history = conversation_history(
            &record(),
            "client-ab12",
            "milla",
            MAX_MESSAGES,
            utc(NOON),
            REUNION_SECONDS,
            None,
        );

        assert_eq!(history.len(), 5, "two texts and three rows: {history:?}");
        assert_eq!(history[0].role, "system");
        assert!(
            history[0]
                .content
                .starts_with("What this conversation knows about the person:"),
            "the profile is the most durable thing here: {:?}",
            history[0]
        );
        assert_eq!(
            history[1],
            row(
                "system",
                "Conversation summary memory:\n- We talked about the shed."
            )
        );
        assert_eq!(
            history[2..],
            [
                row("user", "first"),
                row("assistant", "second"),
                row("user", "third"),
            ]
        );
    }

    #[test]
    fn the_cap_applies_to_the_tail_and_to_nothing_else() {
        // A cap of two keeps the newest two rows; a cap of zero keeps none of them and
        // leaves the two memory messages alone, which is the whole point of the
        // profile and the summary being messages of their own.
        let two = conversation_history(
            &record(),
            "client-ab12",
            "",
            2,
            utc(NOON),
            REUNION_SECONDS,
            None,
        );
        assert_eq!(
            two[two.len() - 2..],
            [row("assistant", "second"), row("user", "third")]
        );
        assert_eq!(two.len(), 4);

        let none = conversation_history(
            &record(),
            "client-ab12",
            "",
            0,
            utc(NOON),
            REUNION_SECONDS,
            None,
        );
        assert_eq!(none.len(), 2, "profile and memory only: {none:?}");
        assert!(none[0].content.starts_with("What this conversation"));
        assert!(none[1].content.starts_with("Conversation summary memory:"));
    }

    #[test]
    fn a_memory_is_sent_only_when_there_is_one() {
        let bare = ConversationRecord {
            messages: vec![message("user", "hello", "2026-10-27T11:58:00+00:00")],
            summaries: Vec::new(),
            delivery_style: None,
            user_details: None,
            user_details_covered_through: None,
        };

        let history = conversation_history(
            &bare,
            "client-ab12",
            "",
            MAX_MESSAGES,
            utc(NOON),
            REUNION_SECONDS,
            None,
        );
        assert_eq!(history, [row("user", "hello")]);
    }

    #[test]
    fn with_a_cache_the_two_texts_become_one_named_block() {
        let mut plan = PromptCachePlan::new("browser-1", "");

        let history = conversation_history(
            &record(),
            "client-ab12",
            "milla",
            MAX_MESSAGES,
            utc(NOON),
            REUNION_SECONDS,
            Some(&mut plan),
        );

        // One system message, and it carries the placeholder rather than the texts.
        assert_eq!(history.len(), 4, "one block and three rows: {history:?}");
        assert_eq!(history[0].role, "system");
        assert!(
            history[0].content.starts_with("{{cache:user_info_")
                && history[0].content.ends_with("}}"),
            "a placeholder, not the text: {}",
            history[0].content
        );

        // The block's text is the two texts in the order they would have been sent,
        // held under the name this conversation and this character share.
        let held: Vec<&str> = plan.blocks.values().map(String::as_str).collect();
        assert_eq!(held.len(), 1, "{held:?}");
        assert!(
            held[0].starts_with("What this conversation knows about the person:"),
            "{}",
            held[0]
        );
        assert!(
            held[0].contains("\n\nConversation summary memory:"),
            "the two texts join with a blank line: {}",
            held[0]
        );
        let named = plan.blocks.keys().next();
        assert_eq!(
            named.map(String::as_str),
            Some(shed_prompts::blocks::user_info_block_key("client-ab12", "milla").as_str())
        );
    }

    #[test]
    fn with_a_cache_and_nothing_to_say_the_history_is_the_tail_alone() {
        let mut plan = PromptCachePlan::new("browser-1", "");
        let bare = ConversationRecord {
            messages: vec![message("user", "hello", "2026-10-27T11:58:00+00:00")],
            summaries: Vec::new(),
            delivery_style: None,
            user_details: None,
            user_details_covered_through: None,
        };

        let history = conversation_history(
            &bare,
            "client-ab12",
            "",
            MAX_MESSAGES,
            utc(NOON),
            REUNION_SECONDS,
            Some(&mut plan),
        );

        assert_eq!(history, [row("user", "hello")]);
        assert!(plan.blocks.is_empty(), "no block for nothing: {plan:?}");
    }

    #[test]
    fn no_key_names_no_history() {
        // The source's first line, and the reason an unnamed conversation cannot name
        // a block either: `user_info_block_key` is empty without a conversation.
        let history = conversation_history(
            &record(),
            "",
            "",
            MAX_MESSAGES,
            utc(NOON),
            REUNION_SECONDS,
            None,
        );
        assert!(history.is_empty());
    }
}
