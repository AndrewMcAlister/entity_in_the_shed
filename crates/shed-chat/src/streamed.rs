//! Reconciling a finished reply with the sentences already spoken from it.
//!
//! Port of `_streamed_prefix_length`, `split_reply_after_stream` and
//! `strip_streamed_prefix` from `agent.py:5942-5985`.
//!
//! ## The problem these three solve
//!
//! A streamed turn is spoken in pieces and then returned as one document. The
//! transcript must show the reply without saying it twice, and the audio must not
//! be replayed on top of itself. Both are the same question: how much of the
//! final reply did the user already hear?
//!
//! ## Why whitespace is skipped and nothing else is
//!
//! [`crate::SentenceSplitter`] strips every sentence it releases, so runs of
//! spaces and newlines legitimately differ between the spoken text and the
//! reply. Everything else must match exactly — a single differing character means
//! the text the user heard is not the text being returned, at which point there
//! is no honest way to subtract it, and the answer is `None`: keep the whole
//! reply, and for [`split_reply_after_stream`] report that the reply was
//! rewritten after it started being spoken.
//!
//! ## Character indices
//!
//! [`streamed_prefix_length`] returns a *character* index, matching Python's
//! `len(reply)` arithmetic. Callers slice with it through
//! [`reply_after_prefix`], which is the port's one place where that index meets
//! Rust's byte-indexed strings.

use crate::py_whitespace_char;

/// How much of `reply` the already-spoken `spoken` covers, or `None`.
///
/// Whitespace is not compared: the splitter strips each sentence, so runs of
/// spaces and newlines legitimately differ between the two. Everything else must
/// match exactly, because anything else means the text the user heard is not the
/// text being returned.
#[must_use]
pub fn streamed_prefix_length(reply: &str, spoken: &str) -> Option<usize> {
    // Indexed by character for the same reason Python indexes `reply` by
    // character: the returned number is a position in that sequence.
    let reply: Vec<char> = reply.chars().collect();
    let reply_length = reply.len();
    let mut reply_index = 0usize;
    for character in spoken.chars() {
        if py_whitespace_char(character) {
            continue;
        }
        // Step over the whitespace the splitter stripped out of the spoken text.
        while reply_index < reply_length && py_whitespace_char(reply[reply_index]) {
            reply_index += 1;
        }
        if reply_index >= reply_length || reply[reply_index] != character {
            return None;
        }
        reply_index += 1;
    }
    Some(reply_index)
}

/// Return `(remaining, replaced)` for a reply that was partly streamed.
///
/// `remaining` is the part the user has not heard. `replaced` reports that the
/// streamed text is not a prefix of the final reply at all — the reply was
/// rewritten after it started being spoken, so the audio and the transcript
/// disagree and the count is worth watching.
pub fn split_reply_after_stream(reply: &str, spoken: &str) -> (String, bool) {
    match streamed_prefix_length(reply, spoken) {
        None => (reply.to_owned(), !spoken.trim().is_empty()),
        Some(prefix) => (reply_after_prefix(reply, prefix), false),
    }
}

/// Drop a leading copy of text the user has already heard.
pub fn strip_streamed_prefix(reply: &str, spoken: &str) -> String {
    match streamed_prefix_length(reply, spoken) {
        None => reply.to_owned(),
        Some(prefix) => reply_after_prefix(reply, prefix),
    }
}

/// `reply[prefix_length:].lstrip()` — `prefix` counts characters, not bytes.
fn reply_after_prefix(reply: &str, prefix: usize) -> String {
    reply
        .chars()
        .skip(prefix)
        .skip_while(|character| py_whitespace_char(*character))
        .collect()
}
