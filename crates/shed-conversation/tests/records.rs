//! The round-trip that pins the absent-optional-field rule.
//!
//! `How_To_Code` principle 15, and the one thing `SLICE-4a-BRIEF` decision 1 calls
//! out as silently gettable wrong: the source's three `NotRequired` fields must load
//! from a record written before they existed **and** be written back absent, not
//! `null`. A `null` is a value, and the loader slice 4b writes would then have to
//! distinguish `null` from absent to tell "the profile is empty" from "there has
//! never been a profile" — a distinction the file on disk does make.
//!
//! Lives in its own integration test rather than in `record.rs`'s module because it
//! is about the *stored bytes*: the record type and its JSON are one claim, and this
//! is the test that says so.

// The test binary links these crates through the library but names none of them
// itself; the workspace's `unused_crate_dependencies` lint judges the binary on its
// own uses.
use serde as _;
use shed_store as _;

use std::collections::BTreeMap;

use shed_conversation::{ConversationMessage, ConversationRecord, ConversationSummary};

/// The oldest shape a conversation record has ever had on disk: two keys.
const OLD_RECORD: &str = r#"{"messages":[{"role":"user","content":"question 1","timestamp":"2026-10-27T12:00:00+00:00"}],"summaries":[{"start_timestamp":"2026-10-20T09:00:00+00:00","end_timestamp":"2026-10-26T18:00:00+00:00","summary":"We talked about the shed."}]}"#;

/// The same record after a turn that set a style and one idle pass that filled in a
/// profile — the three extra keys, in the order the source writes them.
const CURRENT_RECORD: &str = r#"{"messages":[{"role":"user","content":"question 1","timestamp":"2026-10-27T12:00:00+00:00"}],"summaries":[{"start_timestamp":"2026-10-20T09:00:00+00:00","end_timestamp":"2026-10-26T18:00:00+00:00","summary":"We talked about the shed."}],"delivery_style":"warm","user_details":{"name":"Andrew"},"user_details_covered_through":"2026-10-27T12:00:00+00:00"}"#;

#[test]
fn a_record_written_before_the_optional_fields_existed_reads_and_writes_without_them(
) -> Result<(), serde_json::Error> {
    let record: ConversationRecord = serde_json::from_str(OLD_RECORD)?;

    assert_eq!(
        record.messages,
        vec![ConversationMessage {
            role: "user".to_owned(),
            content: "question 1".to_owned(),
            timestamp: "2026-10-27T12:00:00+00:00".to_owned(),
        }]
    );
    assert_eq!(
        record.summaries,
        vec![ConversationSummary {
            start_timestamp: "2026-10-20T09:00:00+00:00".to_owned(),
            end_timestamp: "2026-10-26T18:00:00+00:00".to_owned(),
            summary: "We talked about the shed.".to_owned(),
        }]
    );
    assert_eq!(record.delivery_style, None);
    assert_eq!(record.user_details, None);
    assert_eq!(record.user_details_covered_through, None);

    // Absent, not null: rewriting an old record must not add the keys it never had.
    assert_eq!(serde_json::to_string(&record)?, OLD_RECORD);

    // A record that does carry all three keeps them, and reads back unchanged.
    let mut details = BTreeMap::new();
    details.insert("name".to_owned(), "Andrew".to_owned());
    let current = ConversationRecord {
        messages: record.messages,
        summaries: record.summaries,
        delivery_style: Some("warm".to_owned()),
        user_details: Some(details),
        user_details_covered_through: Some("2026-10-27T12:00:00+00:00".to_owned()),
    };
    let written = serde_json::to_string(&current)?;

    assert_eq!(written, CURRENT_RECORD);
    assert_eq!(
        serde_json::from_str::<ConversationRecord>(&written)?,
        current
    );
    Ok(())
}
