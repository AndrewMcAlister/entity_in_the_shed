//! What one housekeeping pass trims, and what it summarizes before trimming.
//!
//! Ported from `utils/conversation_history.py:375-445`
//! (`plan_conversation_compaction`). Pure in the source's own sense: the record and
//! the clock arrive as arguments, a plan leaves, and nothing is written. The caller
//! that owns the file decides what to do with the plan — slice 4d — and the plan's
//! [`ConversationCompactionPlan::consumed_count`] is what makes that write safe: the
//! housekeeper removes exactly the rows this pass read, so a turn that lands while
//! the summary is being written is not dropped along with it.
//!
//! ## Two cuts, and both take a prefix
//!
//! One by **age** — the detailed window, past which the exact transcript stops being
//! exact — and one by **message count**, which is the cap the person asked for. The
//! later cut wins, because material past either bound is past the promise.
//!
//! `detailed_days <= 0` is the source's `else` and it means *trim everything by age*,
//! not keep everything: a window of no days is a window of no days. `max_messages`
//! defers to [`message_window_start_index`] for the same reading of its own cap.
//!
//! ## What is summarized is deliberately not what is trimmed
//!
//! Material pushed out by the **message cap** is always summarized, even with the
//! summary memory switched off (`SUMMARY_MEMORY_DAYS=0` is the shipped default): the
//! cap is what the user asked for, and dropping the overflow would lose conversation
//! the cap exists to preserve. Age-trimmed material is summarized only inside the
//! summary window; older than that it is dropped, as it always was.
//!
//! ## The two edges worth naming
//!
//! Retained summaries are pruned by `end_timestamp` **only when the summary window
//! is on** — with the memory off, the summary the message overflow just wrote would
//! otherwise be deleted by the very write that created it. And they are sorted by
//! that stamp read as a **string**, as the source sorts it: ISO-8601 orders
//! correctly lexicographically, so no parse is needed and none of the
//! [`Timestamp`] rules apply. A stamp in another grammar sorts as text all the same.
//!
//! The rows the source guards with `if not isinstance(message, dict)` arrive already
//! typed here, so the skips disappear — the row shape settles where the file is
//! parsed, which is a `docs/PARITY.md` row from slice 4b.

use chrono::{DateTime, Duration, Utc};

use crate::record::{
    ConversationCompactionPlan, ConversationMessage, ConversationRecord, ConversationSummary,
};
use crate::timestamp::{is_at_or_after, Timestamp};
use crate::window::message_window_start_index;

/// `timedelta(days=…)` as a [`Timestamp`], the way the source builds its cutoffs.
///
/// Python's `timedelta` takes a float, so a setting of `7.5` days is 7 and a half
/// rather than 7 or 8, and the milliseconds are what carry that: rounding to whole
/// days would move the boundary by up to a day for a setting the source honours.
/// A `days` too large to be a span saturates rather than wrapping, because a cutoff
/// that wrapped into the future would trim a whole transcript.
fn cutoff(now: DateTime<Utc>, days: f64) -> Timestamp {
    let span = Duration::milliseconds((days * 86_400_000.0) as i64);
    Timestamp::Aware((now - span).fixed_offset())
}

/// How many rows at the front fall outside the detailed window.
///
/// The source's loop, which stops at the first row that is at or after the cutoff:
/// the transcript is stored oldest first, so the rows to trim are a prefix and a row
/// that has aged out is never followed by one that has not. A row whose stamp cannot
/// be read is recent — reading it as old is the one direction that lets a single
/// malformed row delete a conversation.
fn aged_out_prefix(messages: &[ConversationMessage], detailed_cutoff: Timestamp) -> usize {
    messages
        .iter()
        .take_while(|row| !is_at_or_after(Some(&row.timestamp), detailed_cutoff))
        .count()
}

/// Whether one trimmed row should be folded into a summary before it goes.
///
/// The message cap's overflow always is (its index is inside the prefix the cap
/// cut); the age-trimmed remainder only inside the summary window.
fn is_worth_a_summary(
    row: &ConversationMessage,
    index: usize,
    message_cut: usize,
    summary_cutoff: Option<Timestamp>,
) -> bool {
    index < message_cut
        || summary_cutoff.is_some_and(|cutoff| is_at_or_after(Some(&row.timestamp), cutoff))
}

/// The summary blocks the record keeps, oldest block first.
///
/// Pruned by `end_timestamp` only inside a summary window; with the memory off every
/// block survives, including the one written for the message overflow. The sort is
/// on the stored text, as the source sorts it, so a block whose stamp cannot be read
/// still sits where its spelling puts it rather than dropping out.
fn retained_summaries(
    summaries: &[ConversationSummary],
    summary_cutoff: Option<Timestamp>,
) -> Vec<ConversationSummary> {
    let mut kept: Vec<ConversationSummary> = match summary_cutoff {
        Some(cutoff) => summaries
            .iter()
            .filter(|summary| is_at_or_after(Some(&summary.end_timestamp), cutoff))
            .cloned()
            .collect(),
        None => summaries.to_vec(),
    };
    kept.sort_by(|earlier, later| earlier.end_timestamp.cmp(&later.end_timestamp));
    kept
}

/// The last `max_summaries` blocks of an already-sorted run: Python's `[-n:]`.
///
/// The `[-0:]` case is the whole list rather than the empty one, and it is the one
/// value arithmetic gets wrong — `len - 0` is `len`, which would delete a record's
/// entire memory on the pass after an operator unset the cap. So zero is answered
/// directly, and every other count takes the tail it names.
fn newest_kept(kept: &[ConversationSummary], max_summaries: usize) -> &[ConversationSummary] {
    if max_summaries == 0 {
        kept
    } else {
        let from = kept.len().saturating_sub(max_summaries);
        &kept[from..]
    }
}

/// Decide the trimmed record without writing anything.
///
/// Ported from `plan_conversation_compaction` (375-445).
///
/// `max_summaries` is the source's keyword argument, which defaults to 36; a Rust
/// caller states it, which is what the housekeeping slice does anyway when it reads
/// the setting. Keeping the **last** that many is Python's `[-n:]`, including its
/// `[-0:]` quirk: a cap of zero keeps every block rather than none.
///
/// `retained_messages` are the rows from the cut on, as copies — the plan never
/// takes the record's own rows, because the caller still needs the record it read in
/// order to remove exactly `consumed_count` of them.
#[must_use]
pub fn plan_conversation_compaction(
    record: &ConversationRecord,
    now: DateTime<Utc>,
    detailed_days: f64,
    summary_days: f64,
    max_messages: i64,
    max_summaries: usize,
) -> ConversationCompactionPlan {
    let messages = record.messages.as_slice();

    let age_cut = if detailed_days > 0.0 {
        aged_out_prefix(messages, cutoff(now, detailed_days))
    } else {
        messages.len()
    };
    let message_cut = message_window_start_index(messages.len(), max_messages);
    let cut = age_cut.max(message_cut);

    let summary_cutoff = (summary_days > 0.0).then(|| cutoff(now, summary_days));

    let messages_to_summarize = messages[..cut]
        .iter()
        .enumerate()
        .filter(|(index, row)| is_worth_a_summary(row, *index, message_cut, summary_cutoff))
        .map(|(_, row)| row.clone())
        .collect();

    let kept = retained_summaries(&record.summaries, summary_cutoff);

    ConversationCompactionPlan {
        messages_to_summarize,
        retained_messages: messages[cut..].to_vec(),
        retained_summaries: newest_kept(&kept, max_summaries).to_vec(),
        consumed_count: cut,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::plan_conversation_compaction;
    use crate::record::{
        ConversationCompactionPlan, ConversationMessage, ConversationRecord, ConversationSummary,
    };

    // `PlanCompactionTests` (`test_conversation_history_utils.py:379-514`).
    //
    // The fixtures choose their cutoffs so that the age cut and the message cut
    // disagree on purpose: a plan that got the two mixed up still keeps thirty rows
    // and still consumes a prefix, and only the contents say which cut it used.
    //
    // The source's `assertIsInstance(result, ConversationCompactionPlan)` is the
    // return type here, so it is not restated.

    /// The source's `NOW`.
    fn now() -> DateTime<Utc> {
        match Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).single() {
            Some(moment) => moment,
            None => panic!("the fixture's clock is not a moment"),
        }
    }

    /// The source's `stamp(days_ago=…)`: `now - timedelta(days=…)`, spelled the way
    /// `isoformat()` spells it. Whole days from a whole-hour clock keep the fraction
    /// absent, exactly as Python's does.
    fn stamp(days_ago: i64) -> String {
        (now() - chrono::Duration::days(days_ago)).to_rfc3339()
    }

    fn row(role: &str, content: &str, timestamp: &str) -> ConversationMessage {
        ConversationMessage {
            role: role.to_owned(),
            content: content.to_owned(),
            timestamp: timestamp.to_owned(),
        }
    }

    /// One interaction: a user message and the one assistant beat that answers it,
    /// both stamped `days_ago`.
    fn exchange(index: usize, days_ago: i64) -> Vec<ConversationMessage> {
        let moment = stamp(days_ago);
        vec![
            row("user", &format!("question {index}"), &moment),
            row("assistant", &format!("answer {index}.0"), &moment),
        ]
    }

    /// The source's `transcript(*exchanges)`: exchanges laid end to end.
    fn transcript(exchanges: Vec<Vec<ConversationMessage>>) -> Vec<ConversationMessage> {
        exchanges.into_iter().flatten().collect()
    }

    /// The source's `*(exchange(index) for index in range(1, 32))`: 31 exchanges, so
    /// 62 rows against a 30-row window.
    fn many_exchanges(count: usize) -> Vec<Vec<ConversationMessage>> {
        (1..=count).map(|index| exchange(index, 0)).collect()
    }

    fn record(
        messages: Vec<ConversationMessage>,
        summaries: Vec<ConversationSummary>,
    ) -> ConversationRecord {
        ConversationRecord {
            messages,
            summaries,
            delivery_style: None,
            user_details: None,
            user_details_covered_through: None,
        }
    }

    /// A remembered block covering one day, as the housekeeper stores one.
    fn remembered(days_ago: i64, text: &str) -> ConversationSummary {
        let moment = stamp(days_ago);
        ConversationSummary {
            start_timestamp: moment.clone(),
            end_timestamp: moment,
            summary: text.to_owned(),
        }
    }

    /// The source's `plan(…)`: a record with no summaries and the source's own cap of
    /// 36, with the three windows stated because Rust has no keyword arguments.
    fn plan(
        messages: Vec<ConversationMessage>,
        detailed_days: f64,
        summary_days: f64,
        max_messages: i64,
    ) -> ConversationCompactionPlan {
        plan_conversation_compaction(
            &record(messages, Vec::new()),
            now(),
            detailed_days,
            summary_days,
            max_messages,
            36,
        )
    }

    fn contents(rows: &[ConversationMessage]) -> Vec<&str> {
        rows.iter().map(|row| row.content.as_str()).collect()
    }

    fn remembered_texts(blocks: &[ConversationSummary]) -> Vec<&str> {
        blocks.iter().map(|block| block.summary.as_str()).collect()
    }

    #[test]
    fn a_conversation_under_the_cap_is_left_alone() {
        let messages = transcript(vec![exchange(1, 0), exchange(2, 0)]);

        let outcome = plan(messages.clone(), 7.0, 0.0, 30);

        assert_eq!(outcome.consumed_count, 0);
        assert_eq!(outcome.retained_messages, messages);
        assert!(outcome.messages_to_summarize.is_empty());
    }

    #[test]
    fn rows_past_the_message_window_are_summarized_even_with_summary_memory_off() {
        // The shipped default is SUMMARY_MEMORY_DAYS=0, where nothing used to be
        // summarized at all. The cap is the promise, so its overflow has to be.
        let messages = transcript(many_exchanges(31));

        let outcome = plan(messages, 7.0, 0.0, 30);

        // 62 rows, a 30-row window, and nothing older than the seven-day window, so
        // the message cut is the one that wins.
        assert_eq!(outcome.consumed_count, 32);
        let summarized = contents(&outcome.messages_to_summarize);
        assert_eq!(&summarized[..2], ["question 1", "answer 1.0"]);
        assert_eq!(summarized.len(), 32);
        assert_eq!(outcome.retained_messages.len(), 30);
        assert_eq!(outcome.retained_messages[0].content, "question 17");
    }

    #[test]
    fn material_older_than_the_summary_window_is_dropped_without_a_summary() {
        let messages = transcript(vec![exchange(1, 40), exchange(2, 10), exchange(3, 0)]);

        let outcome = plan(messages, 7.0, 30.0, 30);

        // The age cut reaches four rows, and only the pair that is old enough to
        // leave the exact transcript and young enough to be remembered is summarized.
        assert_eq!(outcome.consumed_count, 4);
        assert_eq!(
            contents(&outcome.messages_to_summarize),
            ["question 2", "answer 2.0"]
        );
        assert_eq!(
            contents(&outcome.retained_messages),
            ["question 3", "answer 3.0"]
        );
    }

    #[test]
    fn a_missing_timestamp_counts_as_recent() {
        // A row that cannot prove it is old is not old. Reading it the other way lets
        // one malformed row delete a conversation. `""` is what a required timestamp
        // field spells when there is no stamp, which is what Python's missing key is.
        let moment = stamp(0);
        let messages = vec![
            row("user", "no stamp", ""),
            row("assistant", "no stamp either", ""),
            row("user", "question 1", &moment),
            row("assistant", "answer 1.0", &moment),
        ];

        let outcome = plan(messages, 7.0, 0.0, 30);

        assert_eq!(outcome.consumed_count, 0);
        assert!(outcome
            .retained_messages
            .iter()
            .any(|row| row.role == "user" && row.content == "no stamp"));
    }

    #[test]
    fn a_zero_detailed_window_trims_everything() {
        let messages = transcript(vec![exchange(1, 0)]);
        let count = messages.len();

        let outcome = plan(messages, 0.0, 0.0, 30);

        assert_eq!(outcome.consumed_count, count);
        assert!(outcome.retained_messages.is_empty());
        // And trimmed without a summary: the message cap cut nothing here, and the
        // summary memory is off.
        assert!(outcome.messages_to_summarize.is_empty());
    }

    #[test]
    fn summaries_are_kept_when_summary_memory_is_disabled() {
        // Otherwise the summary written for the message overflow would be deleted by
        // the very write that created it.
        let record = record(transcript(many_exchanges(31)), vec![remembered(1, "old")]);

        let outcome = plan_conversation_compaction(&record, now(), 7.0, 0.0, 30, 36);

        assert_eq!(remembered_texts(&outcome.retained_summaries), ["old"]);
    }

    #[test]
    fn summaries_ending_outside_the_window_are_pruned() {
        let record = record(
            transcript(vec![exchange(1, 0)]),
            vec![remembered(40, "expired"), remembered(2, "recent")],
        );

        let outcome = plan_conversation_compaction(&record, now(), 7.0, 30.0, 30, 36);

        assert_eq!(remembered_texts(&outcome.retained_summaries), ["recent"]);
    }

    #[test]
    fn the_newest_summaries_are_kept_up_to_the_limit() {
        // Written oldest-first by the file, so the sort is what puts them back in
        // order before the last two are kept.
        let blocks = vec![
            remembered(5, "5"),
            remembered(4, "4"),
            remembered(3, "3"),
            remembered(2, "2"),
            remembered(1, "1"),
        ];
        let record = record(Vec::new(), blocks);

        let outcome = plan_conversation_compaction(&record, now(), 7.0, 0.0, 30, 2);

        assert_eq!(remembered_texts(&outcome.retained_summaries), ["2", "1"]);
    }

    #[test]
    fn no_cap_keeps_every_summary_as_python_slice_minus_zero_does() {
        // Python's `[-0:]` is the whole list, not the empty one, so a cap of zero
        // keeps every block. Read as `len - 0` it would silently delete a record's
        // whole memory, which is the opposite of what an unset cap means.
        let blocks = vec![remembered(3, "3"), remembered(2, "2"), remembered(1, "1")];
        let record = record(Vec::new(), blocks);

        let outcome = plan_conversation_compaction(&record, now(), 7.0, 0.0, 30, 0);

        assert_eq!(
            remembered_texts(&outcome.retained_summaries),
            ["3", "2", "1"]
        );
    }

    #[test]
    fn the_age_cut_wins_when_it_reaches_further_than_the_message_cut() {
        let messages = transcript(vec![exchange(1, 40), exchange(2, 30), exchange(3, 0)]);

        let outcome = plan(messages, 7.0, 0.0, 30);

        // The message cap would cut nothing, so the four rows consumed are the age
        // cut's — and with the summary memory off, none of them is summarized.
        assert_eq!(outcome.consumed_count, 4);
        assert!(outcome.messages_to_summarize.is_empty());
    }
}
