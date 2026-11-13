//! The three rules that read the freshness of a stored transcript.
//!
//! Ported from `utils/conversation_history.py:242-309` — `unprofiled_messages`
//! (242), `newest_message_timestamp` (272) and `conversation_reopened` (293). All
//! three are pure: the clock arrives as an argument, no file is opened, and the
//! rows arrive as the typed [`ConversationMessage`] rather than as raw JSON.
//!
//! ## The rule all three share: unreadable is recent
//!
//! A row whose stamp cannot be read is kept, counted, and treated as the newest
//! thing in the file. Every direction that guesses at an unknown stamp loses
//! pass or an unnecessary greeting. The source states this in its own docstrings,
//! and all three rules inherit it from [`Timestamp::parse`] answering `None`.
//!
//! ## Rows are typed here, not tolerated
//!
//! The source guards every loop with `if not isinstance(message, dict): continue`,
//! because it reads the record straight out of JSON and any row could be a string
//! or a number. This crate's functions take `&[ConversationMessage]`, so the row
//! shape is settled where the file is parsed (slice 4d) and a malformed row fails
//! **there** rather than being skipped silently here — recorded in
//! `docs/PARITY.md`.
//!
//! The same typing moves one more thing: `ConversationMessage::timestamp` is a
//! required field, so "a row with no timestamp" is spelled `""` in Rust where
//! Python saw a missing key. Both read as no stamp, because neither parses and the
//! rule above decides what that means.

use std::time::Duration;

use crate::record::ConversationMessage;
use crate::timestamp::Timestamp;

/// The rows a profile pass has not read yet, oldest first.
///
/// Ported from `unprofiled_messages` (242-277).
///
/// `covered_through` is the timestamp of the newest message an earlier pass gave
/// the model, not the time that pass ran. Comparing against it rather than
/// counting rows is what lets the marker survive trimming: rows leave from the
/// front, and the marker names a row that stays. `None` — the field a record
/// written before the profile existed does not carry — means nothing has been read,
/// so every row is handed over. `""` is the same case: Python reads both as falsy,
/// and neither parses as a stamp.
///
/// The rows are returned as copies. Python hands back the same dictionaries it was
/// given; a caller that needs to edit rows in place edits the `Vec` it owns, which
/// is what the housekeeping slice does anyway when it trims.
#[must_use]
pub fn unprofiled_messages(
    messages: &[ConversationMessage],
    covered_through: Option<&str>,
) -> Vec<ConversationMessage> {
    let Some(marker) = Timestamp::parse(covered_through) else {
        return messages.to_vec();
    };
    messages
        .iter()
        .filter(|message| {
            // A row whose timestamp cannot be read counts as unread: skipping it
            // is the one direction that loses something the person said.
            Timestamp::parse(Some(&message.timestamp)).is_none_or(|parsed| parsed.after(marker))
        })
        .cloned()
        .collect()
}

/// The newest readable stamp in a block of rows, spelled exactly as it was read.
///
/// Ported from `newest_message_timestamp` (272-291).
///
/// The original spelling comes back rather than a re-serialised datetime, so the
/// marker a caller stores compares against later rows exactly as the file spells
/// them — including a row written by an older version of the app whose offset or
/// separator differs from what this one would write. `None` is what Python's `""`
/// means: no readable stamp in the block, or no rows at all.
///
/// Rows sharing the newest instant keep the **first** one's spelling, because the
/// source only replaces on a strictly-greater comparison. That is visible whenever
/// a user message and the assistant beats answering it are stamped in the same
/// microsecond, which is what the record writer does for a fast turn.
#[must_use]
pub fn newest_message_timestamp(messages: &[ConversationMessage]) -> Option<&str> {
    let mut newest: Option<(&str, Timestamp)> = None;
    for message in messages {
        let Some(parsed) = Timestamp::parse(Some(&message.timestamp)) else {
            continue;
        };
        if newest.is_none_or(|(_, best)| parsed.after(best)) {
            newest = Some((message.timestamp.as_str(), parsed));
        }
    }
    newest.map(|(value, _)| value)
}

/// Whether the stored conversation has been quiet long enough to be a return.
///
/// Ported from `conversation_reopened` (293-309).
///
/// A conversation is keyed by the browser and never ends, so "coming back" is the
/// gap since its last message rather than a new file. A record with no readable
/// stamp is not a return: there is no earlier visit to return from.
///
/// `quiet_seconds` of zero disables the greeting rather than making every turn a
/// reunion, as the source's `<= 0` does. A Rust `Duration` cannot carry a negative,
/// so a caller reading a config value maps anything below zero to zero and gets the
/// same answer.
///
/// The stamp is read as a string and then parsed again, which is the source's own
/// double read and is kept on purpose: the string is the marker the housekeeping
/// slice stores in `user_details_covered_through`, so what is compared here is
/// exactly what the file will hold.
#[must_use]
pub fn conversation_reopened(
    messages: &[ConversationMessage],
    now: Timestamp,
    quiet_seconds: Duration,
) -> bool {
    if quiet_seconds.is_zero() {
        return false;
    }
    let Some(newest) =
        newest_message_timestamp(messages).and_then(|value| Timestamp::parse(Some(value)))
    else {
        return false;
    };
    newest.gap_at_least(now, quiet_seconds)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{conversation_reopened, newest_message_timestamp, unprofiled_messages};
    use crate::record::ConversationMessage;
    use crate::timestamp::Timestamp;

    /// A row as the record writer stores one; `""` is Python's missing timestamp.
    fn message(content: &str, timestamp: &str) -> ConversationMessage {
        ConversationMessage {
            role: String::from("user"),
            content: String::from(content),
            timestamp: String::from(timestamp),
        }
    }

    fn stamp(text: &str) -> Timestamp {
        if let Some(parsed) = Timestamp::parse(Some(text)) {
            return parsed;
        }
        panic!("{text} should parse as a stamp")
    }

    const TWELVE: &str = "2026-10-27T12:00:00+00:00";
    const TWELVE_TEN: &str = "2026-10-27T12:10:00+00:00";
    const TWELVE_TWENTY: &str = "2026-10-27T12:20:00+00:00";

    fn contents(messages: &[ConversationMessage]) -> Vec<&str> {
        messages.iter().map(|row| row.content.as_str()).collect()
    }

    #[test]
    fn no_marker_means_everything_is_unread() {
        let messages = [message("one", TWELVE), message("two", TWELVE_TEN)];

        // No marker and Python's "" are the same case: nothing has been read.
        assert_eq!(
            contents(&unprofiled_messages(&messages, None)),
            ["one", "two"]
        );
        assert_eq!(
            contents(&unprofiled_messages(&messages, Some(""))),
            ["one", "two"]
        );
    }

    #[test]
    fn only_rows_after_the_marker_are_unread() {
        let messages = [
            message("old", TWELVE),
            message("read", TWELVE_TEN),
            message("new", TWELVE_TWENTY),
        ];

        let unread = unprofiled_messages(&messages, Some(TWELVE_TEN));

        assert_eq!(contents(&unread), ["new"]);
    }

    #[test]
    fn a_row_without_a_timestamp_counts_as_unread() {
        // Skipping it is the one direction that loses something the person said.
        let messages = [message("no stamp", ""), message("new", TWELVE_TWENTY)];

        assert_eq!(
            contents(&unprofiled_messages(&messages, Some(TWELVE_TEN))).len(),
            2
        );
    }

    #[test]
    fn the_newest_timestamp_is_read_from_the_tail() {
        let messages = [
            message("old", TWELVE),
            message("new", TWELVE_TWENTY),
            message("no stamp", ""),
        ];

        // The original spelling comes back, and an unreadable row never wins.
        assert_eq!(newest_message_timestamp(&messages), Some(TWELVE_TWENTY));
        assert_eq!(newest_message_timestamp(&[]), None);
    }

    #[test]
    fn the_first_spelling_wins_when_two_rows_share_an_instant() {
        // The source replaces only on strictly greater, so a fast turn's user row
        // keeps its spelling as the marker.
        let messages = [
            message("user", "2026-10-27T12:00:00+00:00"),
            message("reply", "2026-10-27T12:00:00Z"),
        ];

        assert_eq!(
            newest_message_timestamp(&messages),
            Some("2026-10-27T12:00:00+00:00")
        );
    }

    #[test]
    fn a_gap_at_least_as_long_as_the_threshold_is_a_return() {
        let messages = [message("last thing said", "2026-10-27T11:00:00+00:00")];

        assert!(conversation_reopened(
            &messages,
            stamp(TWELVE),
            Duration::from_secs(3600)
        ));
    }

    #[test]
    fn a_recent_message_is_not_a_return() {
        let messages = [message("last thing said", "2026-10-27T11:59:00+00:00")];

        assert!(!conversation_reopened(
            &messages,
            stamp(TWELVE),
            Duration::from_secs(3600)
        ));
    }

    #[test]
    fn zero_disables_the_greeting() {
        let messages = [message("last thing said", "2026-10-26T12:00:00+00:00")];

        assert!(!conversation_reopened(
            &messages,
            stamp(TWELVE),
            Duration::ZERO
        ));
    }

    #[test]
    fn a_record_with_no_readable_stamp_is_not_a_return() {
        // There is no earlier visit to return from.
        assert!(!conversation_reopened(
            &[],
            stamp(TWELVE),
            Duration::from_secs(3600)
        ));
        assert!(!conversation_reopened(
            &[message("no stamp", "")],
            stamp(TWELVE),
            Duration::from_secs(3600)
        ));
    }
}
