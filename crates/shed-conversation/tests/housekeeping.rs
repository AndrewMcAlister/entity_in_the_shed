//! What the idle sweep does to a conversation it finds quiet: trims it, remembers it,
//! and leaves alone the ones it must not touch.
//!
//! All twenty-five tests of `tests/test_conversation_housekeeping.py`, in the brief's order and
//! in five batches of five.
//!
//! `ConversationHousekeepingTests` (57-525) is the first twenty-one: whether a quiet
//! conversation is tidied at all, what a failed summary or a failed profile read costs, which
//! files beside it are not conversations, what the profile marker does and does not let a pass
//! read, and what a background profile worker may and may not cost the turn that started it.
//! `UserProfileRequestTests` (544-602) is the last four, and asks the narrower question — what
//! the one call that decides what is remembered about a person is shown, and what it does with
//! an answer that turns out not to be a profile.
//!
//! ## The seam moved down one layer, and why that is the only change
//!
//! The Python patches two of `agent`'s own functions — `summarize_conversation_messages`
//! and `update_conversation_user_details` — and so never runs a prompt builder. This crate
//! calls both directly from the pass, so there is nothing to patch from outside it; the
//! seam is the [`BackgroundModel`] they both call, and one [`FakeModel`] stands in for both
//! patches at once, telling them apart by the `purpose` string each passes.
//!
//! That is a lower seam rather than a different one. `call_count` becomes a count of the
//! calls carrying each purpose. `call_args.args[0]`, which was the list of rows handed to
//! the summarizer, becomes the transcript those rows were rendered into — so the two
//! assertions about it are made on the rendered lines: fifty-two of them, beginning with
//! the first exchange. What the prompts *say* is pinned by `housekeeping.rs`'s own unit
//! tests and by batch 5; what is pinned here is which rows walked into the prompt, and what
//! the file looks like afterwards.
//!
//! Batch 5 sits the other way round. The Python patches `call_lm_http_with_retry`, one layer
//! *below* the function under test, so its prompt builder really runs; the Rust test calls
//! [`update_conversation_user_details`] with a [`FakeModel`] standing where that mock stood,
//! and its builder runs too. Same depth, same prompt, and the four assertions about the
//! prompt's text carry over unchanged.
//!
//! The other two substitutions are the ones the resume notes name: `tempfile` is not a
//! dependency of this crate, so [`scratch`] builds and empties its own directory the way
//! `tests/store.rs` does; and the workspace builds `chrono` without its `clock` feature, so
//! the clock is a value — the source's fixed `NOW`, passed in — rather than something read.
//!
//! The `patch.object(agent, …)` of the seven settings is [`settings`], which states the
//! reference suite's numbers (`detailed 7 days, no age cut, a cap of ten`) rather than
//! borrowing [`HousekeepingSettings::default`]: its cap is 240, which would trim nothing
//! these fixtures are built to have trimmed.

// The test binary links these crates through the library but names none of them
// itself; the workspace's `unused_crate_dependencies` lint judges the binary on its
// own uses.
use serde as _;
use shed_prompts as _;
use shed_store as _;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use shed_conversation::{
    normalize_user_details, profile_conversation_worker, read_conversation_record_file,
    run_housekeeping, schedule_conversation_profile_update, update_conversation_profile,
    update_conversation_user_details, write_conversation_record_file, BackgroundModel, ChatMessage,
    ConversationMessage, ConversationRecord, ConversationRoot, HousekeepingSettings,
    HousekeepingState, ModelError, ProfileWorkers, SweepFailure, UserDetails,
    CHARACTER_OUTLINE_FILENAME, PROFILE_PURPOSE, SUMMARY_PURPOSE,
};

/// The source's `NOW`: fixed rather than read from the clock, so a test run at midnight
/// behaves the same as one at noon.
fn now() -> DateTime<Utc> {
    match Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).single() {
        Some(moment) => moment,
        None => panic!("the fixture's clock is not a moment"),
    }
}

/// The sweep's `CONVERSATION_IDLE_SECONDS`, and the age every fixture states rather than
/// waits for: six hundred is comfortably past it, thirty is comfortably short of it.
const IDLE_SECONDS: u64 = 300;
/// `CONVERSATION_COOLDOWN_SECONDS` — how long a record that failed is then left alone.
const COOLDOWN_SECONDS: u64 = 900;

/// The seven settings `setUp` patches, as one value.
///
/// `max_messages: 10` is the one that matters: the default cap is 240, and a sweep run
/// with it consumes nothing, so every fixture here would pass for the wrong reason.
fn settings() -> HousekeepingSettings {
    HousekeepingSettings {
        detailed_days: 7.0,
        summary_days: 0.0,
        max_messages: 10,
        max_summaries: 36,
        idle: Duration::from_secs(IDLE_SECONDS),
        cooldown: Duration::from_secs(COOLDOWN_SECONDS),
        poll: Duration::from_secs(30),
    }
}

/// `stamp(seconds_ago=…)`: the source's fixed clock moved back, spelled the way
/// `isoformat()` spells a whole second.
fn stamp(seconds_ago: i64) -> String {
    (now() - chrono::Duration::seconds(seconds_ago)).to_rfc3339()
}

fn row(role: &str, content: &str, timestamp: &str) -> ConversationMessage {
    ConversationMessage {
        role: role.to_owned(),
        content: content.to_owned(),
        timestamp: timestamp.to_owned(),
    }
}

/// One interaction: a user message and the beats answering it, each row carrying its own
/// stamp — the source's reason being that a fixture where every row shared one would make
/// every row look covered.
fn exchange(index: i64, beats: usize) -> Vec<ConversationMessage> {
    let seconds_ago = (40 - index) * 10;
    let mut rows = vec![row(
        "user",
        &format!("question {index}"),
        &stamp(seconds_ago),
    )];
    for beat in 0..beats {
        let offset = i64::try_from(beat).unwrap_or(i64::MAX);
        rows.push(row(
            "assistant",
            &format!("answer {index}.{beat}"),
            &stamp(seconds_ago - 1 - offset),
        ));
    }
    rows
}

/// `transcript(*exchanges)`: the exchanges laid end to end, oldest first.
fn transcript(
    exchanges: impl IntoIterator<Item = Vec<ConversationMessage>>,
) -> Vec<ConversationMessage> {
    exchanges.into_iter().flatten().collect()
}

/// The thirty-one exchanges the trimming fixtures fill a sixty-two-row transcript with.
fn thirty_one_exchanges() -> Vec<ConversationMessage> {
    transcript((1..=31).map(|index| exchange(index, 1)))
}

/// The record `write_conversation` builds: the transcript, no summaries, and the style the
/// conversation was spoken in when there is one.
fn stored(messages: Vec<ConversationMessage>, delivery_style: Option<&str>) -> ConversationRecord {
    ConversationRecord {
        messages,
        summaries: Vec::new(),
        delivery_style: delivery_style.map(str::to_owned),
        user_details: None,
        user_details_covered_through: None,
    }
}

/// A memory root of this test's own, emptied first so a crashed earlier run cannot leave a
/// file that changes what an assertion sees.
fn scratch(label: &str) -> ConversationRoot {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let memory_dir = std::env::temp_dir().join(format!(
        "shed-conversation-housekeeping-{label}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&memory_dir);
    std::fs::create_dir_all(&memory_dir)
        .unwrap_or_else(|error| panic!("cannot make {}: {error}", memory_dir.display()));
    ConversationRoot { memory_dir }
}

/// Take the run's scratch directory away again.
fn tidy(root: &ConversationRoot) {
    let _ = std::fs::remove_dir_all(&root.memory_dir);
}

/// Where a bare address's record lives: `memory_dir/<filename>`, which is the shape of
/// every path in these tests.
fn record_path(root: &ConversationRoot, filename: &str) -> PathBuf {
    root.record_path(filename, None)
}

/// `write_conversation(…, idle_seconds=600)`: the record is written and then aged, so
/// "idle" is a fact the test states rather than one the clock arranges.
fn write_idle(path: &Path, record: &ConversationRecord) {
    write_record(path, record);
    touch(path, as_system_time(now() - chrono::Duration::seconds(600)));
}

/// The same for a conversation written half a minute ago — the one a sweep must not read.
fn write_recent(path: &Path, record: &ConversationRecord) {
    write_record(path, record);
    touch(path, as_system_time(now() - chrono::Duration::seconds(30)));
}

/// `write_conversation(…, idle_seconds=0)`: a conversation as warm as it can be, which is
/// what one that has just finished a turn looks like to everything except the profile pass.
/// The four batch-4 tests are about the profile being written *now*, so their fixtures are
/// files the idle sweep would refuse outright.
fn write_warm(path: &Path, record: &ConversationRecord) {
    write_record(path, record);
    touch(path, as_system_time(now()));
}

fn write_record(path: &Path, record: &ConversationRecord) {
    write_conversation_record_file(path, record)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

/// `read_conversation`: the record as the file holds it now, read back rather than
/// trusted from whatever was last written to it.
fn read(path: &Path) -> ConversationRecord {
    read_conversation_record_file(path)
}

/// Set a file's modified time, so its age is the test's and not the filesystem's.
fn touch(path: &Path, moment: SystemTime) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|error| panic!("cannot open {}: {error}", path.display()));
    file.set_modified(moment)
        .unwrap_or_else(|error| panic!("cannot stamp {}: {error}", path.display()));
}

/// The instant the file was last written to — the mtime the sweep remembers it by, and
/// the moment a later sweep is measured from.
fn modified(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|meta| meta.modified())
        .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
}

/// The file's bytes, for the assertion a parsed comparison cannot make: that a pass which
/// decided to do nothing did not touch the file at all.
fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The sweep's own reduction of a `DateTime` to a `SystemTime`, so a fixture and the code
/// under test disagree about nothing.
fn as_system_time(moment: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH
        + Duration::from_secs(moment.timestamp().max(0) as u64)
        + Duration::from_nanos(u64::from(moment.timestamp_subsec_nanos()))
}

/// The other direction, for a `now` the test reads off the file or off the cooldown map.
fn as_utc(moment: SystemTime) -> DateTime<Utc> {
    let since = moment
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("the fixture's clock ran backwards: {error}"));
    let Ok(seconds) = i64::try_from(since.as_secs()) else {
        panic!("the fixture's clock is out of a datetime's range");
    };
    match DateTime::from_timestamp(seconds, since.subsec_nanos()) {
        Some(moment) => moment,
        None => panic!("the fixture's clock is out of a datetime's range"),
    }
}

/// How one of the two background calls answers.
#[derive(Clone)]
enum Answer {
    /// The transport refused: the source's `status_code >= 400`, and the reason
    /// `summarize_conversation_messages` raises rather than falling back.
    Failed,
    /// The `choices[0].message.content` string the body carries.
    Content(String),
}

/// A `BackgroundModel` that answers without a model, and remembers every prompt it was
/// shown.
///
/// The Python's two `patch`es in one object: the purpose string each caller passes is what
/// tells them apart, which is exactly what the real transport needs the same string for. A
/// fresh fake stands in for each sweep, as a fresh `MagicMock` does there.
struct FakeModel {
    summary_answer: Answer,
    profile_answer: Answer,
    /// Profile answers still queued, oldest first, one per call: the source's
    /// `answers = iter([…])` handed to a `side_effect` callable. While it holds anything,
    /// every profile call answers the next one and never `profile_answer`.
    profile_answers: Mutex<VecDeque<Answer>>,
    /// Run before answering, and so the seam a turn is made to land *during* a call.
    on_summary: Box<dyn Fn() + Send + Sync>,
    on_profile: Box<dyn Fn() + Send + Sync>,
    /// Every prompt, in the order it arrived: `(purpose, the whole message list)`.
    asked: Mutex<Vec<(String, Vec<ChatMessage>)>>,
}

impl FakeModel {
    /// The reference suite's defaults: the summarizer answers `"A summary."` and the
    /// profiler answers with an empty profile.
    ///
    /// The Python's profiler mock returns `{}` — the normalized details dict. Here the
    /// answer is a model *body*, and a body whose content carries no `user_details` object
    /// is `ModelError::NotJson` rather than an empty profile, so the empty answer is
    /// spelled as the prompt's own schema asks for it.
    fn new() -> Self {
        Self {
            summary_answer: Answer::Content(r#"{"summary": "A summary."}"#.to_owned()),
            profile_answer: Answer::Content(r#"{"user_details": {}}"#.to_owned()),
            profile_answers: Mutex::new(VecDeque::new()),
            on_summary: Box::new(|| ()),
            on_profile: Box::new(|| ()),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// The summary call answers this instead — `"A summary."`, `"They carried on."`,
    /// `"Recovered."`: the model's own words, coming back out of the record.
    fn with_summary(mut self, text: &str) -> Self {
        self.summary_answer = Answer::Content(format!(r#"{{"summary": "{text}"}}"#));
        self
    }

    /// The summary call fails: the source's `side_effect=RuntimeError(…)`.
    fn with_failing_summary(mut self) -> Self {
        self.summary_answer = Answer::Failed;
        self
    }

    /// The profile call answers this: `profile_return_value=…`, the dict the Python's mock
    /// hands back, spelled as the prompt's own schema asks so it survives `reply_content`.
    fn with_profile(mut self, details: &str) -> Self {
        self.profile_answer = Answer::Content(format!(r#"{{"user_details": {details}}}"#));
        self
    }

    /// The profile call answers this exact content, with no `user_details` wrapper around it:
    /// the replies that are not profiles at all. `with_profile` spells a *profile*; batch 5
    /// needs the three answers that are not one — a summary where a profile was asked for, a
    /// sentence of prose, and no answer whatsoever.
    fn with_raw_profile(mut self, content: &str) -> Self {
        self.profile_answer = Answer::Content(content.to_owned());
        self
    }

    /// The profile call answers *these*, one per call, oldest first: the source's
    /// `answers = iter([{…}, {…}])` fed to a `side_effect` callable.
    ///
    /// Only one caller asks twice — the worker, which loops while the record still holds
    /// unread rows — and it asks twice precisely because the second call was shown a turn the
    /// first never saw. One answer for both would let a worker that looped without re-reading
    /// look correct.
    fn with_profile_answers<'a>(self, answers: impl IntoIterator<Item = &'a str>) -> Self {
        let spelt: Vec<Answer> = answers
            .into_iter()
            .map(|details| Answer::Content(format!(r#"{{"user_details": {details}}}"#)))
            .collect();
        // The guard is scoped rather than held to the end of the body: the builder hands `self`
        // back, and the borrow that fills the queue would still be alive when it moves.
        {
            let Ok(mut queued) = self.profile_answers.lock() else {
                panic!("the fake's own answer queue was poisoned by a panicking test");
            };
            queued.extend(spelt);
        }
        self
    }

    /// The profile call fails: the source's `profile_side_effect=RuntimeError(…)`.
    fn with_failing_profile(mut self) -> Self {
        self.profile_answer = Answer::Failed;
        self
    }

    /// A side effect on the profile call: the source's `profile_side_effect`, which is a
    /// *callable* there rather than an exception. The one test that passes one appends a turn
    /// and answers with a profile, so it pairs this with `with_profile` — Python spells both
    /// as one `side_effect` because a callable there is allowed to answer as well as act.
    fn on_profile_call<F>(mut self, effect: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.on_profile = Box::new(effect);
        self
    }

    /// The side effect the Python's `side_effect` callable is. It runs while the call is
    /// in progress, which is the whole point of the test that passes one.
    fn on_summary_call<F>(mut self, effect: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.on_summary = Box::new(effect);
        self
    }

    /// The next profile answer: a queued one while the queue holds any, so the fake built for
    /// a looping worker answers differently each time it is asked, and `profile_answer` once
    /// the queue is empty — which is what every other test in this file relies on.
    fn next_profile_answer(&self) -> Answer {
        let Ok(mut queued) = self.profile_answers.lock() else {
            return self.profile_answer.clone();
        };
        queued
            .pop_front()
            .unwrap_or_else(|| self.profile_answer.clone())
    }

    /// How many calls carried this purpose — `call_count` on the matching mock.
    fn count(&self, purpose: &str) -> usize {
        self.prompts(purpose).len()
    }

    /// Every message list shown for this purpose, oldest call first.
    fn prompts(&self, purpose: &str) -> Vec<Vec<ChatMessage>> {
        let Ok(asked) = self.asked.lock() else {
            panic!("the fake's own log was poisoned by a panicking test");
        };
        asked
            .iter()
            .filter(|(name, _)| name == purpose)
            .map(|(_, messages)| messages.clone())
            .collect()
    }

    /// The lines of the one prompt this purpose got — the transcript the rows became,
    /// which is what `call_args.args[0]` turned into. Panics on no call or on a prompt
    /// with no user message in it, because every caller of this asserts the count first.
    fn only_prompt_lines(&self, purpose: &str) -> Vec<String> {
        let mut prompts = self.prompts(purpose);
        let Some(messages) = prompts.pop() else {
            panic!("{purpose} was never asked");
        };
        let Some(last) = messages.last() else {
            panic!("a {purpose} prompt is never empty");
        };
        last.content.lines().map(str::to_owned).collect()
    }
}

impl BackgroundModel for FakeModel {
    fn complete_json(
        &self,
        purpose: &str,
        messages: &[ChatMessage],
    ) -> Result<serde_json::Value, ModelError> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push((purpose.to_owned(), messages.to_vec()));
        }
        // Owned rather than borrowed, because a queued answer is taken out of the queue and
        // so has no home to borrow from.
        let (answer, effect) = if purpose == SUMMARY_PURPOSE {
            (self.summary_answer.clone(), &self.on_summary)
        } else {
            (self.next_profile_answer(), &self.on_profile)
        };
        effect();
        match answer {
            Answer::Failed => Err(ModelError::Transport("model unavailable".to_owned())),
            Answer::Content(content) => Ok(serde_json::json!({
                "choices": [{"message": {"content": content}}],
            })),
        }
    }
}

/// `run_sweep`: one pass over the store with a fresh fake, the state as it stands, and the
/// chat counter at the zero the source's tests leave it at.
fn run_sweep(
    root: &ConversationRoot,
    state: &mut HousekeepingState,
    moment: DateTime<Utc>,
    model: &FakeModel,
) -> Vec<(PathBuf, SweepFailure)> {
    run_housekeeping(root, &settings(), state, 0, moment, model)
}

/// The `summary` of every block the record remembers, oldest block first.
fn remembered(record: &ConversationRecord) -> Vec<&str> {
    record
        .summaries
        .iter()
        .map(|summary| summary.summary.as_str())
        .collect()
}

/// What every row the record still holds exactly says.
fn contents(record: &ConversationRecord) -> Vec<&str> {
    record
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect()
}

#[test]
fn an_idle_conversation_past_the_cap_is_trimmed_and_summarized_once() {
    let root = scratch("trim-once");
    let path = record_path(&root, "192.0.2.50.json");
    let messages = thirty_one_exchanges();
    write_idle(&path, &stored(messages.clone(), None));
    let mut state = HousekeepingState::default();

    let sweep = FakeModel::new();
    run_sweep(&root, &mut state, now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    let summarized = sweep.only_prompt_lines(SUMMARY_PURPOSE);
    assert_eq!(summarized.len(), 52);
    assert_eq!(
        summarized[..2],
        ["user: question 1", "assistant: answer 1.0"]
    );

    let record = read(&path);
    assert_eq!(record.messages.len(), 10);
    assert_eq!(record.messages[0].content, "question 27");
    assert_eq!(remembered(&record), ["A summary."]);
    assert_eq!(record.summaries[0].start_timestamp, messages[0].timestamp);
    // Row 51 is the last one the summary covered: the sixty-two rows minus the ten the
    // record keeps, which is the same prefix the trim dropped.
    assert_eq!(record.summaries[0].end_timestamp, messages[51].timestamp);
    assert!(state.checked.contains_key(&path));
    // The whole conversation was new to the profile, and its marker names the newest row
    // the model was shown.
    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        messages.last().map(|message| message.timestamp.as_str())
    );

    // A second sweep over the same bytes must not pay for a second summary.
    let later = as_utc(modified(&path) + Duration::from_secs(IDLE_SECONDS + 1));
    let again = FakeModel::new();
    run_sweep(&root, &mut state, later, &again);

    assert_eq!(again.count(SUMMARY_PURPOSE), 0);
    assert_eq!(again.count(PROFILE_PURPOSE), 0);
    assert_eq!(read(&path), record);
    tidy(&root);
}

#[test]
fn trimming_a_conversation_keeps_the_style_it_was_spoken_in() {
    // Housekeeping rewrites the record. The style beside the transcript is part of the
    // conversation, so the rewrite must carry it rather than being how it is lost.
    let root = scratch("trim-style");
    let path = record_path(&root, "192.0.2.57.json");
    write_idle(&path, &stored(thirty_one_exchanges(), Some("spooky")));

    let sweep = FakeModel::new();
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    let record = read(&path);
    assert_eq!(record.messages.len(), 10);
    assert_eq!(record.delivery_style.as_deref(), Some("spooky"));
    tidy(&root);
}

#[test]
fn a_recently_written_conversation_is_left_alone() {
    let root = scratch("recently-written");
    let path = record_path(&root, "192.0.2.51.json");
    let messages = thirty_one_exchanges();
    write_recent(&path, &stored(messages.clone(), None));

    let sweep = FakeModel::new();
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    let record = read(&path);
    assert_eq!(record.messages, messages);
    assert!(record.summaries.is_empty());
    tidy(&root);
}

#[test]
fn a_turn_appended_during_the_summary_is_kept() {
    // The summary is a model call, and a person can send another message while it runs.
    // The trim drops the rows the plan read, not simply the oldest rows in whatever the
    // file holds by then.
    let root = scratch("turn-during-summary");
    let path = record_path(&root, "192.0.2.52.json");
    let messages = thirty_one_exchanges();
    write_idle(&path, &stored(messages.clone(), None));
    let appended = path.clone();
    let append_a_turn = move || {
        let mut record = read(&appended);
        record.messages.extend(exchange(32, 1));
        write_record(&appended, &record);
    };

    let sweep = FakeModel::new()
        .with_summary("They carried on.")
        .on_summary_call(append_a_turn);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    let record = read(&path);
    let kept = contents(&record);
    assert_eq!(kept.len(), 12);
    assert_eq!(kept[0], "question 27");
    assert_eq!(kept[kept.len() - 2], "question 32");
    assert_eq!(kept[kept.len() - 1], "answer 32.0");
    assert_eq!(remembered(&record), ["They carried on."]);
    // The turn landed after the profile read: its rows are not claimed.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        messages.last().map(|message| message.timestamp.as_str())
    );
    tidy(&root);
}

#[test]
fn a_summarizer_failure_leaves_the_conversation_alone() {
    let root = scratch("summary-failure");
    let path = record_path(&root, "192.0.2.53.json");
    let messages = thirty_one_exchanges();
    write_idle(&path, &stored(messages.clone(), None));
    let mut state = HousekeepingState::default();

    let sweep = FakeModel::new().with_failing_summary();
    run_sweep(&root, &mut state, now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    let record = read(&path);
    assert_eq!(record.messages, messages);
    assert!(record.summaries.is_empty());
    let Some(retry_at) = state.retry_at.get(&path).copied() else {
        panic!("a summary that failed should have cost a cooldown");
    };
    assert!(retry_at > as_system_time(now()));

    // The cooldown is what a model that is down gets answered with, and it is a moment
    // rather than a count: the sweep before it is passed over, the one after it runs.
    let still_cooling = FakeModel::new().with_failing_summary();
    run_sweep(
        &root,
        &mut state,
        as_utc(retry_at - Duration::from_secs(60)),
        &still_cooling,
    );
    assert_eq!(still_cooling.count(SUMMARY_PURPOSE), 0);

    let recovered = FakeModel::new().with_summary("Recovered.");
    run_sweep(
        &root,
        &mut state,
        as_utc(retry_at + Duration::from_secs(1)),
        &recovered,
    );
    assert_eq!(recovered.count(SUMMARY_PURPOSE), 1);
    let after = read(&path);
    assert_eq!(remembered(&after), ["Recovered."]);
    // The rows a failed summary refused to write down are the ones still there to be
    // summarized again — nothing was dropped on the way out.
    assert_eq!(after.messages.len(), 10);
    assert_eq!(after.messages[0].content, "question 27");
    tidy(&root);
}

// ---------------------------------------------------------------------------
// Batch 2 — the files the sweep must not touch, and the second job it has
// besides trimming.
// ---------------------------------------------------------------------------

/// A file the sweep cannot read as a record: bytes written straight onto a record's own
/// path and then aged like any other, which is what a half-written save looks like from here.
fn write_damaged(path: &Path, text: &str) {
    std::fs::write(path, text)
        .unwrap_or_else(|error| panic!("cannot damage {}: {error}", path.display()));
    touch(path, as_system_time(now() - chrono::Duration::seconds(600)));
}

/// The character outline a folder carries beside its conversations, written the way the app
/// that owns it writes it — plain JSON, and no part of any conversation.
fn write_outline(folder: &Path, outline: &serde_json::Value) {
    std::fs::create_dir_all(folder)
        .unwrap_or_else(|error| panic!("cannot make {}: {error}", folder.display()));
    let path = folder.join(CHARACTER_OUTLINE_FILENAME);
    std::fs::write(&path, outline.to_string())
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

/// That file read back and parsed, which is how the source asserts it is intact: the bytes
/// may be re-spelled by anything that touches them, and what is asserted is the document.
fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => panic!("{} stopped being JSON: {error}", path.display()),
    }
}

/// The whole message list of the one prompt a purpose got, system message first.
///
/// [`FakeModel::only_prompt_lines`] reads the half carrying the transcript, which is where
/// the rows walk in. This reads the other half, which is where a character's name and the
/// profile as it stands are spelled out. Panics on no call, because every caller of it
/// asserts the count first.
fn only_prompt(sweep: &FakeModel, purpose: &str) -> Vec<ChatMessage> {
    let mut prompts = sweep.prompts(purpose);
    match prompts.pop() {
        Some(messages) => messages,
        None => panic!("{purpose} was never asked"),
    }
}

/// The instruction half of a two-message prompt.
fn system_of(prompt: &[ChatMessage]) -> &str {
    match prompt.first() {
        Some(message) => message.content.as_str(),
        None => panic!("a prompt is never empty"),
    }
}

/// The half of a two-message prompt that carries the profile and the transcript.
fn user_of(prompt: &[ChatMessage]) -> &str {
    match prompt.last() {
        Some(message) => message.content.as_str(),
        None => panic!("a prompt is never empty"),
    }
}

#[test]
fn a_profile_failure_leaves_the_conversation_alone() {
    // The summary may succeed, but the profile read is the only chance to record what the
    // person said before those rows are trimmed away. A failed read therefore costs a
    // retry, not the facts.
    let root = scratch("profile-failure");
    let path = record_path(&root, "192.0.2.58.json");
    let messages = transcript((1..=7).map(|index| exchange(index, 1)));
    // Fourteen rows against a cap of ten, so there is a trim waiting for this failure to
    // stop — which is the whole test: the same fixture with four rows would pass by doing
    // nothing at all.
    assert_eq!(messages.len(), 14);
    write_idle(&path, &stored(messages.clone(), None));
    let mut state = HousekeepingState::default();

    let sweep = FakeModel::new().with_failing_profile();
    let failures = run_sweep(&root, &mut state, now(), &sweep);

    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);
    // The pass stops at the profile rather than carrying on to the summary: the rows the
    // failed read would have learned from are still in the file, and so is the reason they
    // were worth summarizing.
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(failures.len(), 1);
    let record = read(&path);
    assert_eq!(record.messages, messages);
    assert!(record.user_details.is_none());
    let Some(retry_at) = state.retry_at.get(&path).copied() else {
        panic!("a profile read that failed should have cost a cooldown");
    };
    assert!(retry_at > as_system_time(now()));
    tidy(&root);
}

#[test]
fn a_damaged_record_is_left_alone() {
    // Reading a damaged file as the empty record and writing that back would erase the
    // words the damage did not reach.
    let root = scratch("damaged-record");
    let path = record_path(&root, "192.0.2.54.json");
    write_damaged(&path, "{not json");

    let mut state = HousekeepingState::default();
    let sweep = FakeModel::new();
    let failures = run_sweep(&root, &mut state, now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    assert_eq!(
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
        "{not json"
    );
    // Unreadable is not a failure: nothing was rewritten, nothing is owed a cooldown, and
    // the next sweep reads it again and leaves it alone again at the price of one read.
    assert!(failures.is_empty());
    assert!(state.retry_at.is_empty());
    tidy(&root);
}

#[test]
fn no_sweep_runs_while_a_chat_turn_is_in_flight() {
    let root = scratch("chat-in-flight");
    let path = record_path(&root, "192.0.2.55.json");
    let messages = thirty_one_exchanges();
    write_idle(&path, &stored(messages.clone(), None));
    let mut state = HousekeepingState::default();

    // The summarizer shares the resident model with the turn being answered, so a sweep that
    // starts mid-turn returns before it reads a file. That is why the state comes back as
    // empty as it went in: the conversation is still due when the turn ends, rather than
    // marked as examined by a sweep that examined nothing.
    let sweep = FakeModel::new();
    let failures = run_housekeeping(&root, &settings(), &mut state, 1, now(), &sweep);

    assert!(failures.is_empty());
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    assert!(state.checked.is_empty());
    assert_eq!(read(&path).messages, messages);
    tidy(&root);
}

#[test]
fn the_character_outline_is_never_touched() {
    // The outline is the one JSON beside a conversation that is not one. It is also what
    // gives the summary the character's own framing.
    let root = scratch("character-outline");
    let slug = "aria-en-gb";
    let folder = root.memory_dir.join(slug);
    let outline = serde_json::json!({"name": "Aria", "locale": "en-GB"});
    write_outline(&folder, &outline);
    let outline_path = folder.join(CHARACTER_OUTLINE_FILENAME);
    let path = root.record_path("192.0.2.56.json", Some(slug));
    write_idle(&path, &stored(thirty_one_exchanges(), None));
    let mut state = HousekeepingState::default();

    let sweep = FakeModel::new();
    run_sweep(&root, &mut state, now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    assert_eq!(read_json(&outline_path), outline);
    // The folder *was* walked, so what follows is a skip by name rather than a folder nobody
    // opened: the record beside the outline is in the sweep's memory and the outline is not.
    assert!(state.checked.contains_key(&path));
    assert!(!state.checked.contains_key(&outline_path));
    // `call_args.kwargs["character_profile"] == outline`. The pass reads the outline for
    // itself now, and hands the model the one sentence of it the prompt has room for.
    let asked = only_prompt(&sweep, SUMMARY_PURPOSE);
    assert!(
        system_of(&asked).ends_with("This memory belongs to the character Aria (en-GB)."),
        "the summary was asked without the character's own framing: {}",
        system_of(&asked)
    );
    tidy(&root);
}

#[test]
fn a_conversation_under_the_cap_still_learns_who_the_person_is() {
    // Trimming is not the only reason to look at a conversation. A short one still has a
    // name and a family worth remembering, and the name is what a returning visit is
    // greeted by.
    let root = scratch("under-the-cap");
    let path = record_path(&root, "192.0.2.59.json");
    let messages = transcript([exchange(1, 1), exchange(2, 1)]);
    write_idle(&path, &stored(messages.clone(), None));

    let sam = r#"{"name": "Sam", "family": "their mum is unwell"}"#;
    let sweep = FakeModel::new().with_profile(sam);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    // Four rows under a cap of ten: nothing to trim, so nothing to summarize, and still a
    // profile to be had.
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);
    // The two arguments the source asserts on — `args[0]`, the rows it was shown, and
    // `args[1]`, the `{}` profile they are being folded into — are the two halves of the one
    // message the worker builds.
    let asked = only_prompt(&sweep, PROFILE_PURPOSE);
    assert_eq!(
        user_of(&asked),
        "Current profile:\n{}\n\nNew messages:\n\
         user: question 1\nassistant: answer 1.0\nuser: question 2\nassistant: answer 2.0"
    );

    let record = read(&path);
    assert_eq!(record.messages, messages);
    let details = match record.user_details.as_ref() {
        Some(details) => details,
        None => panic!("the profile the model answered with was not stored"),
    };
    // The source compares whole dicts, and a dict comparison ignores order; `get` asks the
    // same question without caring which way the parsed document happened to iterate.
    assert_eq!(details.get("name"), Some("Sam"));
    assert_eq!(details.get("family"), Some("their mum is unwell"));
    assert_eq!(details.len(), 2);
    // The marker names the newest row the model was shown, which for a conversation this
    // short is the last row it has.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        messages.last().map(|message| message.timestamp.as_str())
    );
    tidy(&root);
}

// ---------------------------------------------------------------------------
// Batch 3 — the marker. Which rows a profile pass may read, what it owes a
// record that has no profile at all, and what it must not claim to have read.
// ---------------------------------------------------------------------------

/// A profile spelled the way a record's file spells it, read through the loader's own
/// normalizing rather than handed over as a struct: these fixtures are files first.
fn profile(spelling: &str) -> UserDetails {
    let parsed: serde_json::Value = match serde_json::from_str(spelling) {
        Ok(value) => value,
        Err(error) => panic!("not a profile as a file would hold it ({spelling}): {error}"),
    };
    normalize_user_details(&parsed)
}

/// `write_conversation(…, user_details=…, user_details_covered_through=…)`: the record of a
/// conversation some earlier pass has already read up to `covered_through`.
///
/// `None` for `details` is the half these six tests live on — a record carrying a marker with
/// no profile beside it, which is what a conversation written before the profile existed
/// looks like.
fn profiled(
    record: &ConversationRecord,
    details: Option<UserDetails>,
    covered_through: Option<&str>,
) -> ConversationRecord {
    ConversationRecord {
        user_details: details,
        user_details_covered_through: covered_through.map(str::to_owned),
        ..record.clone()
    }
}

/// The stamp of the newest row, which is where a pass that read everything leaves the marker.
fn last_stamp(messages: &[ConversationMessage]) -> &str {
    match messages.last() {
        Some(message) => message.timestamp.as_str(),
        None => panic!("a fixture transcript is never empty"),
    }
}

/// The stamp of one row by position, for a marker left partway through a conversation.
fn stamp_at(messages: &[ConversationMessage], index: usize) -> &str {
    match messages.get(index) {
        Some(message) => message.timestamp.as_str(),
        None => panic!("the fixture transcript has no row {index}"),
    }
}

/// The profile the record holds. Asking for it *is* the assertion these tests make most
/// often: a record that holds none has nothing left to answer with.
fn learned(record: &ConversationRecord) -> &UserDetails {
    match record.user_details.as_ref() {
        Some(details) => details,
        None => panic!("the record holds no profile at all"),
    }
}

/// A profile as `(topic, phrase)` pairs, sorted, for the whole-profile assertions the source
/// makes on whole dicts — and a dict comparison ignores order, so this one does too.
///
/// Sorting is not tidiness. This crate keeps a profile in the *document's* order when it is
/// loaded from a file and in the reply's key order when it is normalized out of a model answer
/// — the asymmetry `profile.rs` records and `tests/records.rs` pins — so an assertion on the
/// order of this list would be an assertion about which side of that seam the value happened
/// to come from. What these tests are about is which facts a profile holds.
fn facts(details: &UserDetails) -> Vec<(&str, &str)> {
    let mut pairs: Vec<(&str, &str)> = details
        .pairs()
        .iter()
        .map(|(label, text)| (label.as_str(), text.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs
}

/// The four rows two exchanges make, oldest first — the short conversation three of these
/// six fixtures are built from.
fn two_exchanges() -> Vec<ConversationMessage> {
    transcript([exchange(1, 1), exchange(2, 1)])
}

#[test]
fn a_missing_profile_is_backfilled_from_the_whole_conversation() {
    // A marker cannot speak for a section that is not there. Whatever this record said it had
    // already read, a missing profile is rebuilt from everything the conversation holds.
    let root = scratch("profile-backfill");
    let path = record_path(&root, "192.0.2.66.json");
    let messages = two_exchanges();
    let read_so_far = last_stamp(&messages);
    write_idle(
        &path,
        &profiled(&stored(messages.clone(), None), None, Some(read_so_far)),
    );

    let sam = r#"{"name": "Sam", "family": "a sick mum"}"#;
    let sweep = FakeModel::new().with_profile(sam);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);
    // The marker's own claim is that every one of these rows was already read. The absence
    // beside it outranks it, because a marker is a promise about a section that has to exist
    // for anything to have been learned.
    let asked = only_prompt(&sweep, PROFILE_PURPOSE);
    assert_eq!(
        user_of(&asked),
        "Current profile:\n{}\n\nNew messages:\n\
         user: question 1\nassistant: answer 1.0\nuser: question 2\nassistant: answer 2.0"
    );

    let record = read(&path);
    assert_eq!(
        facts(learned(&record)),
        [("family", "a sick mum"), ("name", "Sam")]
    );
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(read_so_far)
    );
    tidy(&root);
}

#[test]
fn a_missing_profile_is_written_even_with_nothing_to_ask_about() {
    // Reply beats alone teach nothing, but the section's absence is the problem here. Write
    // it empty rather than leave the next visit reading the record as a stranger's.
    let root = scratch("profile-write-empty");
    let path = record_path(&root, "192.0.2.67.json");
    // One row, and an assistant beat at that: nobody has said anything about themselves.
    let messages = exchange(1, 1).split_off(1);
    write_idle(&path, &stored(messages.clone(), None));

    let sweep = FakeModel::new();
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    let record = read(&path);
    // `{}`, and present: asking for the profile above is the assertion that it was written,
    // and this is the assertion that it was written empty rather than guessed at.
    assert!(learned(&record).is_empty());
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(last_stamp(&messages))
    );
    tidy(&root);
}

#[test]
fn a_profile_only_reads_messages_it_has_not_seen() {
    let root = scratch("profile-marker-window");
    let path = record_path(&root, "192.0.2.60.json");
    let first = two_exchanges();
    let second = exchange(3, 1);
    let messages = transcript([first.clone(), second.clone()]);
    let read_so_far = last_stamp(&first);
    write_idle(
        &path,
        &profiled(
            &stored(messages.clone(), None),
            Some(profile(r#"{"name": "Sam"}"#)),
            Some(read_so_far),
        ),
    );

    let sam = r#"{"name": "Sam", "town_or_city": "Bristol"}"#;
    let sweep = FakeModel::new().with_profile(sam);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    // The rows it was shown are the ones after the marker, and the profile it was shown is
    // the whole thing rather than a diff — a fact the person does not mention again is a fact
    // that survives this call only by being in front of the model.
    let asked = only_prompt(&sweep, PROFILE_PURPOSE);
    assert_eq!(
        user_of(&asked),
        "Current profile:\n{\"name\": \"Sam\"}\n\nNew messages:\n\
         user: question 3\nassistant: answer 3.0"
    );

    let record = read(&path);
    assert_eq!(record.messages, messages);
    assert_eq!(
        facts(learned(&record)),
        [("name", "Sam"), ("town_or_city", "Bristol")]
    );
    // The marker moves to the newest row of *this* read, so the next pass starts here.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(last_stamp(&second))
    );
    tidy(&root);
}

#[test]
fn trimming_keeps_the_profile_it_has_learned() {
    let root = scratch("trim-keeps-profile");
    let path = record_path(&root, "192.0.2.61.json");
    let messages = thirty_one_exchanges();
    let read_so_far = last_stamp(&messages);
    write_idle(
        &path,
        &profiled(
            &stored(messages.clone(), None),
            Some(profile(r#"{"name": "Sam"}"#)),
            Some(read_so_far),
        ),
    );

    let sweep = FakeModel::new();
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    // Every row is behind the marker, so there is nothing left to learn and nothing to ask.
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 1);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);

    let record = read(&path);
    assert_eq!(record.messages.len(), 10);
    assert_eq!(facts(learned(&record)), [("name", "Sam")]);
    // The marker names a row that survived the trim rather than a count of rows: fifty-two
    // rows left the front of this transcript and it still points at the conversation's end.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(read_so_far)
    );
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(last_stamp(&record.messages))
    );
    tidy(&root);
}

#[test]
fn a_turn_that_lands_during_the_profile_is_read_next_time() {
    let root = scratch("turn-during-profile");
    let path = record_path(&root, "192.0.2.62.json");
    let messages = two_exchanges();
    write_idle(&path, &stored(messages.clone(), None));
    let appended = path.clone();
    // The source's `profile_side_effect` is a callable that appends a turn and then answers,
    // so the new rows land while the model is being asked and before the record is written.
    let append_a_turn = move || {
        let mut record = read(&appended);
        record.messages.extend(exchange(3, 1));
        write_record(&appended, &record);
    };

    let sweep = FakeModel::new()
        .with_profile(r#"{"name": "Sam"}"#)
        .on_profile_call(append_a_turn);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);
    let record = read(&path);
    assert_eq!(facts(learned(&record)), [("name", "Sam")]);
    // The marker names the newest row this pass saw, not the newest row in the file: the turn
    // that landed mid-call is still unread, and that is what makes it this pass's business
    // next time rather than lost.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(last_stamp(&messages))
    );
    // The write was built from the read that started the pass, so the only thing keeping
    // those rows is that the record was read again before it was written.
    let kept = contents(&record);
    assert_eq!(kept[kept.len() - 2..], ["question 3", "answer 3.0"]);
    // Six rows, still under the cap: the summary is not what this turn is testing.
    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    tidy(&root);
}

#[test]
fn a_tail_of_reply_beats_alone_does_not_cost_a_profile_call() {
    // Assistant beats teach nothing about the person. They stay unread as context for the
    // next user message rather than buying a model call.
    let root = scratch("beats-alone");
    let path = record_path(&root, "192.0.2.63.json");
    let messages = two_exchanges();
    // The marker stops after `question 2`, so what is left unread is the beat answering it.
    let record = profiled(
        &stored(messages.clone(), None),
        Some(profile(r#"{"name": "Sam"}"#)),
        Some(stamp_at(&messages, 2)),
    );
    write_idle(&path, &record);
    let quiet_for = modified(&path);

    let sweep = FakeModel::new();
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);

    assert_eq!(sweep.count(SUMMARY_PURPOSE), 0);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    // Unread, not covered: the beat is still in front of the marker, waiting to be context
    // for the next thing the person says.
    let after = read(&path);
    assert_eq!(after, record);
    assert_eq!(
        after.user_details_covered_through.as_deref(),
        Some(stamp_at(&messages, 2))
    );
    // A pass that decided there was nothing to do is also a write that did not happen — the
    // file is as old as it was, which is what keeps it due for the next sweep's notice.
    assert_eq!(modified(&path), quiet_for);
    tidy(&root);
}

// ---------------------------------------------------------------------------
// Batch 4 — the profile outside the sweep: written as the turn ends rather than
// waited for, never rewritten when it is already current, looped by a worker that
// re-reads, and scheduled by a turn that must not be charged for any of it.
// ---------------------------------------------------------------------------

/// `update_conversation_profile`'s answer, as the plain yes-or-no the source's `wrote` is.
///
/// A failure is a panic rather than an `assert!` on the `Result`, because these fixtures are
/// built to succeed: `Err` here is the transport refusing or the record being unreadable, and
/// either would make the assertions after it about a file that was never written.
fn wrote_profile(path: &Path, model: &dyn BackgroundModel) -> bool {
    match update_conversation_profile(path, model) {
        Ok(wrote) => wrote,
        Err(error) => panic!("the profile pass failed: {error}"),
    }
}

#[test]
fn the_profile_is_saved_as_the_turn_ends_not_after_the_idle_wait() {
    // The idle wait exists for trimming. The profile is what the next visit is greeted by, so
    // it is written while the conversation is still warm.
    let root = scratch("profile-at-turn-end");
    let path = record_path(&root, "192.0.2.68.json");
    let messages = exchange(1, 1);
    write_warm(&path, &stored(messages.clone(), None));

    // Warm enough that the sweep refuses the file outright, which is the half of the name this
    // fixture cannot otherwise show: the pass that runs as a turn ends is not the idle one, so
    // a warm conversation is one it has already been through.
    let sweep = FakeModel::new().with_profile(r#"{"name": "Sam"}"#);
    run_sweep(&root, &mut HousekeepingState::default(), now(), &sweep);
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);

    assert!(wrote_profile(&path, &sweep));
    assert_eq!(sweep.count(PROFILE_PURPOSE), 1);

    let record = read(&path);
    assert_eq!(facts(learned(&record)), [("name", "Sam")]);
    // The marker names the row it was shown rather than the moment it ran, which is what makes
    // a pass at the end of a turn comparable with one the idle sweep makes a week later.
    assert_eq!(
        record.user_details_covered_through.as_deref(),
        Some(last_stamp(&messages))
    );
    tidy(&root);
}

#[test]
fn an_up_to_date_profile_is_not_rewritten() {
    let root = scratch("profile-up-to-date");
    let path = record_path(&root, "192.0.2.69.json");
    let messages = exchange(1, 1);
    write_warm(
        &path,
        &profiled(
            &stored(messages.clone(), None),
            Some(profile(r#"{"name": "Sam"}"#)),
            Some(last_stamp(&messages)),
        ),
    );
    let before = bytes(&path);

    let sweep = FakeModel::new();
    assert!(!wrote_profile(&path, &sweep));
    assert_eq!(sweep.count(PROFILE_PURPOSE), 0);
    // Byte for byte, which is more than `read` would show: a rewrite would re-serialise the
    // record and move the mtime, and the mtime is what the next sweep measures its idle wait
    // from. A pass that touches a file it decided nothing about keeps a conversation idle past
    // every sweep that follows it.
    assert_eq!(bytes(&path), before);
    tidy(&root);
}

#[test]
fn a_turn_that_lands_during_the_save_is_still_read_immediately() {
    // The worker loops while the record holds unread user messages, so a message typed while
    // the model was answering is not left for idle.
    let root = scratch("turn-during-save");
    let path = record_path(&root, "192.0.2.71.json");
    let messages = exchange(1, 1);
    write_warm(&path, &stored(messages.clone(), None));
    let appended = path.clone();
    let unread = messages.len();
    // The source's `side_effect` callable, including its guard: the turn lands once, while the
    // first answer is being given, and the second call is shown what that left behind.
    let type_a_turn = move || {
        let mut record = read(&appended);
        if record.messages.len() == unread {
            record.messages.extend(exchange(2, 1));
            write_record(&appended, &record);
        }
    };

    let sweep = FakeModel::new()
        .with_profile_answers([
            r#"{"name": "Sam"}"#,
            r#"{"name": "Sam", "likes": "fishing"}"#,
        ])
        .on_profile_call(type_a_turn);
    profile_conversation_worker(&path, &sweep);

    // Two calls, because the worker went round again and found the turn the first pass never
    // saw. One answer for both would let a worker that looped without re-reading pass for one
    // that re-read, so the queue answers each call differently.
    assert_eq!(sweep.count(PROFILE_PURPOSE), 2);
    let record = read(&path);
    assert_eq!(
        facts(learned(&record)),
        [("likes", "fishing"), ("name", "Sam")]
    );
    // The turn itself is still there, and still the newest thing in the file.
    assert_eq!(record.messages.len(), 4);
    let kept = contents(&record);
    assert_eq!(kept[kept.len() - 2..], ["question 2", "answer 2.0"]);
    tidy(&root);
}

/// The same fake behind the two handles a scheduled worker needs: one the test can read a call
/// count through, and one the scheduler can hand to a thread it is not told the type of.
///
/// `Arc::clone` takes its type from what it is being made into, so the coercion to the trait
/// object has to happen at a coercion site — here rather than at either call site.
fn shared_model(model: &Arc<FakeModel>) -> Arc<dyn BackgroundModel> {
    model.clone()
}

/// The source patches `conversation_record_path` to raise `OSError("disk gone")` and asserts
/// only that the call does not propagate. There is no seam to patch here: resolving a record
/// path joins a filename to a folder and touches nothing on disk, so it has no failure to be
/// given. And [`schedule_conversation_profile_update`] returns `()` rather than a `Result`,
/// which is where "never raises into a turn" actually lives in this port — a caller handed
/// nothing cannot propagate it. What is left observable is the other half of the same promise,
/// which is what this test asserts: a conversation with nothing to read costs a turn no model
/// call, no file, and no worker left running behind it.
#[test]
fn the_profile_scheduler_never_raises_into_a_turn() {
    let root = scratch("scheduler-swallows");
    // The key resolves to a record that is not there: the worker's first read finds nothing.
    let path = record_path(&root, "192.0.2.72.json");
    let workers = ProfileWorkers::new();
    let model = Arc::new(FakeModel::new());
    let shared = shared_model(&model);

    // It returned, and there was never anything to return with it.
    schedule_conversation_profile_update(&workers, &root, "192.0.2.72", None, shared);

    assert!(
        workers.wait(Duration::from_secs(5)),
        "a worker it started is still running"
    );
    assert!(workers.is_empty());
    assert!(
        !path.exists(),
        "a conversation with nothing to read was created"
    );
    assert_eq!(model.count(PROFILE_PURPOSE), 0);
    tidy(&root);
}

/// A fake whose profile call blocks until the returned `Sender` is given a message, having
/// first announced itself on the returned `Receiver`.
///
/// This is how "a worker was started" is made checkable *while it is still true*. A worker with
/// nothing to do finishes in microseconds, so checking the set straight after scheduling races
/// the thread and passes for the wrong reason; parking it inside the model call makes the check
/// a fact rather than a timing window.
fn blocked_model(details: &str) -> (FakeModel, Receiver<()>, Sender<()>) {
    let (reached_sender, reached) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    // The channel halves a `Fn` closure needs are `!Sync` on their own, so they travel locked.
    let announced = Mutex::new(reached_sender);
    let barrier = Mutex::new(gate);
    let model = FakeModel::new()
        .with_profile(details)
        .on_profile_call(move || {
            if let Ok(sender) = announced.lock() {
                let _sent = sender.send(());
            }
            if let Ok(bar) = barrier.lock() {
                let _released = bar.recv();
            }
        });
    (model, reached, release)
}

/// The control the refused-key assertion needs: a key the scheduler *does* accept, on a
/// conversation with something to read, parked inside the model call so the worker is provably
/// running at the moment it is looked for.
///
/// Without it, `a_legacy_key_is_not_profiled` would assert that nothing happened in a call that
/// could never have made anything happen either, and would pass just as happily if the scheduler
/// had never been able to start a worker at all.
fn accepted_key_starts_a_worker(root: &ConversationRoot, workers: &ProfileWorkers, warm: &Path) {
    let (control, reached, release) = blocked_model(r#"{"name": "Sam"}"#);
    let control_model = Arc::new(control);
    schedule_conversation_profile_update(
        workers,
        root,
        "192.0.2.70",
        None,
        shared_model(&control_model),
    );

    assert!(
        reached.recv_timeout(Duration::from_secs(5)).is_ok(),
        "an accepted key started no worker at all"
    );
    assert!(workers.is_active(warm), "an accepted key started no worker");
    let _gone = release.send(());
    assert!(
        workers.wait(Duration::from_secs(5)),
        "the control worker never finished"
    );
}

#[test]
fn a_legacy_key_is_not_profiled() {
    let root = scratch("legacy-key");
    let workers = ProfileWorkers::new();

    let warm = record_path(&root, "192.0.2.70.json");
    write_warm(&warm, &stored(exchange(1, 1), None));
    accepted_key_starts_a_worker(&root, &workers, &warm);

    // A session-keyed record from before conversations were named for an address: the key names
    // neither an address nor a browser, so there is no file it could mean, and profiling it
    // would be background work on words nobody reads. The refusal happens before a thread is
    // started rather than inside one, which is the source's `starter.call_count == 0`.
    let model = Arc::new(FakeModel::new());
    schedule_conversation_profile_update(
        &workers,
        &root,
        "browser:legacy-session",
        None,
        shared_model(&model),
    );

    assert!(workers.is_empty(), "a refused key started a worker");
    assert_eq!(model.count(PROFILE_PURPOSE), 0);
    // The control's record is the only thing in the store, so nothing was created for the key
    // that was turned away.
    let names: Vec<String> = std::fs::read_dir(&root.memory_dir)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", root.memory_dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        ["192.0.2.70.json"],
        "a refused key put a file in the store"
    );
    tidy(&root);
}

// ---------------------------------------------------------------------------
// Batch 5 — `UserProfileRequestTests` (544-602): the one call that decides what is
// remembered about a person, and the three answers that turn out not to be one.
//
// This batch is the one the header's note about seam depth is about. The Python patches the
// transport beneath `update_conversation_user_details`, so its prompt builder runs; the Rust
// calls that function with a fake in the transport's place, so this one runs too. Nothing is
// rendered from outside the function under test, which is what lets all four assertions about
// the prompt's own text carry over word for word.
// ---------------------------------------------------------------------------

/// The two rows the source's `setUp` builds: what the person said, and the reply that answered
/// it.
///
/// Both carry the fixed clock's own moment rather than the spaced-out stamps `exchange` gives
/// its rows, because nothing in these four tests compares a stamp. They read what the prompt
/// was made of and what came back; and no record is written here at all, so a marker would be
/// evidence about none of it.
fn one_exchange() -> Vec<ConversationMessage> {
    vec![
        row("user", "my mum is in hospital", &stamp(0)),
        row("assistant", "I'm sorry to hear that.", &stamp(0)),
    ]
}

/// `request_with`: the same two rows and the same one-fact profile, asked of a fake that has
/// already been told how to answer. The four tests differ only in that answer.
///
/// Handing back the `Result` rather than unwrapping it is what lets three of the four assert on
/// the error instead of the answer.
fn ask_for_a_profile(model: &FakeModel) -> Result<UserDetails, ModelError> {
    update_conversation_user_details(&one_exchange(), &profile(r#"{"name": "Sam"}"#), model)
}

#[test]
fn the_model_sees_the_profile_and_the_messages_it_has_not_read() {
    // Three facts, the third of them the long-form `other` the prompt carves out its own budget
    // for. Spelled across lines so the fixture reads as the JSON it is.
    let three_facts = r#"{
        "name": "Sam",
        "family": "their mum is unwell",
        "other": "they are worried about the boiler"
    }"#;
    let model = FakeModel::new().with_profile(three_facts);

    let details = match ask_for_a_profile(&model) {
        Ok(details) => details,
        Err(error) => panic!("a well-formed answer was refused: {error}"),
    };
    assert_eq!(
        facts(&details),
        [
            ("family", "their mum is unwell"),
            ("name", "Sam"),
            ("other", "they are worried about the boiler"),
        ]
    );

    // The source asserts `request["model"] == agent.resolve_background_model()`. Which model
    // answers is the transport's decision and not this crate's, so what carries over is the
    // half that is: the call was made for a profile, and for nothing else.
    assert_eq!(model.count(PROFILE_PURPOSE), 1);
    assert_eq!(model.count(SUMMARY_PURPOSE), 0);

    let prompt = only_prompt(&model, PROFILE_PURPOSE);
    let instruction = system_of(&prompt);
    // The instruction names the labels it wants and spells out the one budget it cannot import
    // — `USER_DETAILS_OTHER_MAX_CHARS`, which a unit test keeps tied to the reader, since the
    // two disagreeing is how a profile ends up holding an `other` the next load silently cuts.
    assert!(instruction.contains("family"), "{instruction}");
    assert!(instruction.contains("\"other\""), "{instruction}");
    assert!(instruction.contains("512"), "{instruction}");

    let body = user_of(&prompt);
    // The profile as it stands, spelled the way the prompt asks for it, and the row that
    // prompted the call. Both have to be in front of the model: the answer replaces the whole
    // profile rather than patching it, so a fact the model is not shown is a fact lost.
    assert!(body.contains(r#""name": "Sam""#), "{body}");
    assert!(body.contains("my mum is in hospital"), "{body}");
}

#[test]
fn an_answer_without_a_profile_raises() {
    // The rows this read covers are about to be trimmed. Answering with the profile it started
    // from would silently drop what the person said instead of leaving the marker where the
    // next quiet spell can try again, which is why this raises rather than falling back.
    let model = FakeModel::new().with_raw_profile(r#"{"summary": "they talked"}"#);
    let error = ask_for_a_profile(&model).err();
    assert!(matches!(error, Some(ModelError::NotJson)), "{error:?}");
    // The refusal came from the answer, not from the fixture: an empty transcript answers with
    // the profile it was given and makes no call at all.
    assert_eq!(model.count(PROFILE_PURPOSE), 1);
}

#[test]
fn a_prose_answer_raises() {
    // A model that answers in sentences. The body arrives whole — an unreadable reply is not a
    // transport failure, since the two callers read one differently — so it is the parse that
    // follows it, and not the call, that refuses.
    let model = FakeModel::new().with_raw_profile("They have a sick mum.");
    let error = ask_for_a_profile(&model).err();
    assert!(matches!(error, Some(ModelError::NotJson)), "{error:?}");
    assert_eq!(model.count(PROFILE_PURPOSE), 1);
}

#[test]
fn an_http_failure_raises() {
    // The source's `status_code >= 400` branch, fed a 500 body: the transport reports the
    // failure it made, and `update_conversation_user_details` passes it straight up rather than
    // treating a call that never happened as one that had nothing new to say.
    let model = FakeModel::new().with_failing_profile();
    let error = ask_for_a_profile(&model).err();
    assert!(matches!(error, Some(ModelError::Transport(_))), "{error:?}");
    assert_eq!(model.count(PROFILE_PURPOSE), 1);
}
