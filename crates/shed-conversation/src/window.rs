//! Where the exact conversation starts.
//!
//! Ported from `utils/conversation_history.py:206-216`
//! (`message_window_start_index`).
//!
//! ## The window is counted in messages, not exchanges
//!
//! A message is a bubble the person saw. A reply is stored as one row per beat, so
//! one exchange of three beats costs three of the ten rows the app promises, and a
//! long answer leaves less room for the exchanges before it. Counting exchanges
//! instead would let the prompt grow without limit on the turns that matter most:
//! the long ones. The cap exists because the prompt is what has to stay small.
//!
//! ## The two edges
//!
//! A cap larger than the transcript starts at 0 — nothing is trimmed, and the
//! function does not pretend otherwise. A cap of `0` or less returns the length,
//! which slices out every row: the caller asked for no window and gets no window,
//! rather than the whole transcript by some accident of `len - 0`.

/// Where the newest `max_messages` rows begin, given how many rows there are.
///
/// `max_messages` is `i64` rather than `usize` because the source's `<= 0` test is
/// meaningful: a caller that reads a config value can bring back a negative cap,
/// and the answer for that is the same as for zero — trim everything. Python's
/// `int(max_messages)` also truncates a float there, which has no counterpart
/// here; the Rust callers pass integers read from settings, so the parameter is an
/// integer and the truncation is nobody's to perform.
///
/// A cap too large for `usize` (only possible where `usize` is narrower than
/// `i64`) means every row fits, which is what `usize::MAX` makes of it below.
#[must_use]
pub fn message_window_start_index(message_count: usize, max_messages: i64) -> usize {
    if max_messages <= 0 {
        return message_count;
    }
    message_count.saturating_sub(usize::try_from(max_messages).unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::message_window_start_index;
    use crate::record::ConversationMessage;

    // `MessageWindowTests` (`test_conversation_history_utils.py:199-227`).
    //
    // The source passes whole transcripts and compares against `len(messages)`;
    // the rows are built here too, because the case worth pinning is which row
    // the window starts *on*, not merely the arithmetic.
    //
    // A plain comment rather than a doc comment: the item that follows is a test
    // helper, and an empty line before it is `clippy::empty_line_after_doc_comments`.
    fn row(role: &str, content: &str) -> ConversationMessage {
        ConversationMessage {
            role: role.to_owned(),
            content: content.to_owned(),
            timestamp: "2026-10-27T12:00:00+00:00".to_owned(),
        }
    }

    /// One interaction: a user message and the assistant beats that answer it.
    fn exchange(index: usize, beats: usize) -> Vec<ConversationMessage> {
        let mut rows = vec![row("user", &format!("question {index}"))];
        rows.extend((0..beats).map(|beat| row("assistant", &format!("answer {index}.{beat}"))));
        rows
    }

    #[test]
    fn the_window_is_the_newest_rows() {
        let messages = [exchange(1, 1), exchange(2, 1), exchange(3, 1)].concat();

        assert_eq!(
            message_window_start_index(messages.len(), 2),
            messages.len() - 2
        );
    }

    #[test]
    fn every_reply_beat_costs_one_of_the_window() {
        // The window bounds the prompt, and each beat is a bubble in it, so a
        // long reply leaves less room for the exchanges before it: four rows of a
        // three-beat answer plus its question push the first exchange right out.
        let messages = [exchange(1, 1), exchange(2, 3)].concat();

        let start = message_window_start_index(messages.len(), 4);

        assert_eq!(messages[start].content, "question 2");
    }

    #[test]
    fn fewer_messages_than_the_cap_keeps_everything() {
        let messages = [exchange(1, 1), exchange(2, 1)].concat();

        assert_eq!(message_window_start_index(messages.len(), 30), 0);
    }

    #[test]
    fn an_empty_transcript_keeps_everything() {
        assert_eq!(message_window_start_index(0, 30), 0);
    }

    #[test]
    fn a_cap_of_zero_trims_everything() {
        let messages = exchange(1, 1);

        assert_eq!(
            message_window_start_index(messages.len(), 0),
            messages.len()
        );
        // And a negative cap, which is what an unset setting reads as, trims the
        // same way rather than everything.
        assert_eq!(
            message_window_start_index(messages.len(), -1),
            messages.len()
        );
    }
}
