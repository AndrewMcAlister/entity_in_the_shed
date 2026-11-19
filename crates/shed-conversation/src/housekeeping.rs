//! The two background model calls, the profile workers, and the idle sweep.
//!
//! Ported from `agent.py:3931-4080` (`summarize_conversation_messages`,
//! `update_conversation_user_details`) and `agent.py:4185-4526`
//! (`load_conversation_record_strict`, `update_conversation_profile`,
//! `profile_conversation_worker`, `start_conversation_profile_update`,
//! `schedule_conversation_profile_update`, `wait_for_conversation_profile_workers`,
//! `conversation_character_profile`, `housekeep_conversation_record_file`,
//! `run_conversation_housekeeping`, and the loop that drives it).
//!
//! ## The model is injected, so this crate still never opens a socket
//!
//! The source calls `call_lm_http_with_retry` with a `purpose=` of
//! `"conversation summary"` or `"conversation user profile"`. This crate's layer
//! position (`shed-gates`'s `LAYERS`) is `config, json, text, store, models, prompts` —
//! `shed-llm` is not in it, and `Cargo.toml` has never carried it: in four slices this
//! crate has assembled prompts and never sent one. So the call is a port rather than a
//! dependency — [`BackgroundModel`] — and the retry ladder, `resolve_background_model`
//! and `LM_ENGINE_HOST` stay with whoever wires the server, which is where the
//! difference between a refused connection and a refused engine is even knowable.
//!
//! The port takes the *parsed body* rather than a status code. The `status_code >= 400`
//! branch and its `_safe_body_snippet` belong to the transport: a caller that has already
//! handed back a parsed body has nothing left to status-code, and a caller that has not
//! returns [`ModelError::Transport`] with the snippet in it. What stays here is
//! `parse_json_response_strict`'s half — read `choices[0].message.content`, then parse
//! *that* as JSON, because both prompts ask for JSON and both read the answer back out of
//! a string.
//!
//! ## The two calls fail in opposite directions, and both are the source's choice
//!
//! The summarizer answers with the last two lines of the transcript when the reply could
//! not be read, because a summary that says almost something is better than a hole where
//! a memory was — and the rows it replaces have already left the exact window. The profile
//! call *raises*, because its caller is about to mark those rows as read: a silent success
//! there would retire what the person said on the strength of a reply that never said it.
//! So the same unreadable reply becomes a fallback string in one function and an `Err` in
//! the other, and neither is a decision made here.
//!
//! ## State is a value, so a test can run two sweeps
//!
//! The source keeps four module globals: the checked mtimes, the retry times, the worker
//! set, and three locks. Here the first two are [`HousekeepingState`], passed `&mut` to
//! [`run_housekeeping`], and the third is [`ProfileWorkers`], a value that owns its set
//! behind one lock. That is decision 6, and it buys two things: a test can run two sweeps
//! that do not see each other's memory, and nothing in this crate can be reached from a
//! thread nobody started. The cost is that the caller now has to *hold* the state — and
//! that two processes re-check everything once each, which is also what the source does
//! across a restart, since its dicts are memory too.
//!
//! ## What is not here
//!
//! The outbox sweep (`agent.py:4485-4526`) is not Stage 4's, and neither is the retention
//! pass or the clearing pass that also use [`ConversationRoot::record_files`]. The loop
//! body is here as [`spawn`], because the brief asks for the poll interval to be a field
//! rather than a `while True` in a binary; nothing starts a thread at import time, and
//! nothing starts one until a caller asks.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::Value;
use shed_store::WriteError;

use crate::compaction::plan_conversation_compaction;
use crate::filename::{conversation_filename, names_a_conversation};
use crate::freshness::{newest_message_timestamp, unprofiled_messages};
use crate::profile::{normalize_user_details, UserDetails};
use crate::record::{
    ConversationCompactionPlan, ConversationMessage, ConversationRecord, ConversationSummary,
};
use crate::store::{write_conversation_record_file, ConversationRoot, CHARACTER_OUTLINE_FILENAME};

/// How much transcript either call is actually shown. Both prompts read
/// `"\n".join(lines)[-12000:]`, which is 12 000 **characters** — Python indexes a `str`,
/// not a `bytes`. One row longer than that loses its own head, and the model is left with
/// the tail of it and nothing saying so.
const TRANSCRIPT_CHAR_BUDGET: usize = 12_000;

/// How many of the transcript's lines the summarizer's fallback is made from.
const FALLBACK_LINE_COUNT: usize = 2;

/// How long that fallback is allowed to be, in characters.
///
/// The comment above the source's `return` says why the number exists: the summary list is
/// read on every later turn, so the fallback must not become the door a whole transcript
/// walks back in through.
const FALLBACK_CHAR_BUDGET: usize = 240;

/// What the summarizer is told to do. Verbatim, including the trailing space before the
/// character sentence: the source concatenates that space with the suffix, so a
/// conversation with no character ends its system prompt with it.
const SUMMARY_SYSTEM: &str = "Summarize the older conversation below for a long-term memory log. \
Return JSON only with the schema {\"summary\": \"...\"}. Write one short, vague sentence of at \
most twenty words: the gist of what was talked about, without names, numbers, decisions or \
details. Durable facts about the person are kept in their profile, so do not repeat them here. ";

/// What the profile worker is told to do. Verbatim from the source's system message.
///
/// The `512` in the last sentence is `USER_DETAILS_OTHER_MAX_CHARS` spelled out in prose,
/// because a prompt cannot import a constant. A unit test pins the two to the same number:
/// the reading side and the asking side disagreeing is how a profile ends up holding a
/// 600-character `other` that the next load silently cuts.
const PROFILE_SYSTEM: &str = "Maintain a short profile of the person this conversation is with. \
Return JSON only with the schema {\"user_details\": {\"label\": \"value\"}}. Record only what the \
person has said about themselves or their preferences: name, age, likes and dislikes, the town \
or city they live in, country, family, and anything else that could matter in a later \
conversation. For example, if they mention a sick mother, add \"family\": \"their mum is \
unwell\". Keep every fact already in the profile unless the new messages correct it. Use short \
lower-case labels; each value is a short phrase, except \"other\", which is for anything that \
does not fit a label and may run to 512 characters. Do not guess.";

/// The `purpose=` string the source passes for a summary call.
pub const SUMMARY_PURPOSE: &str = "conversation summary";

/// The `purpose=` string the source passes for a profile call.
pub const PROFILE_PURPOSE: &str = "conversation user profile";

/// One message of a prompt, as the transport will be asked to send it.
///
/// The same shape `history.rs` assembles for a turn, and deliberately a *separate* type: a
/// turn's history carries the delivery style and the remembered talk that this call has no
/// room for, and a `BackgroundModel` implementor should not have to know which of the two a
/// `role` came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// `"system"` or `"user"`.
    pub role: String,
    /// The text, already assembled.
    pub content: String,
}

impl ChatMessage {
    /// One message.
    #[must_use]
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_owned(),
            content: content.into(),
        }
    }
}

/// A background call that did not produce a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// The call itself failed: no engine, a refused connection, a status the transport
    /// refuses to hand a body back for. This is the source's `status_code >= 400`
    /// `RuntimeError`, and the `_safe_body_snippet` in its message is this string.
    Transport(String),
    /// A body with no usable `choices` array in it. The source has no such case —
    /// `parse_json_response_strict` would have raised first — so an implementor that got a
    /// body of its own choosing reports it here rather than inventing an empty one.
    NoChoice,
    /// A reply whose content was not the JSON the prompt asked for, or JSON with no
    /// `user_details` object in it. The source's two `RuntimeError`s from
    /// `update_conversation_user_details`, which are one case here because no caller can
    /// act on the difference: both mean "leave the record alone".
    NotJson,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(reason) => write!(formatter, "the background call failed: {reason}"),
            Self::NoChoice => write!(formatter, "the reply carried no choices"),
            Self::NotJson => write!(formatter, "the reply was not the JSON asked for"),
        }
    }
}

impl std::error::Error for ModelError {}

/// A housekeeping job that could not finish, and which half of it said so.
///
/// Three variants because the three failures leave different things behind, which is what
/// a log line has to say: a profile failure drops nothing, a summary failure drops nothing
/// and costs a call, and a write failure means the old record is still on disk next to a
/// plan nobody stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HousekeepingError {
    /// The profile call failed, or answered with something that was not a profile. The
    /// record was left exactly as it was, marker included, so the same rows are offered
    /// again on the next quiet spell.
    Profile(ModelError),
    /// The summary call failed. Nothing was dropped, which is the only reason a failed
    /// summary is survivable.
    Summary(ModelError),
    /// A record could not be stored. The reason is the `Display` text of the
    /// [`WriteError`]; the path is known to whoever asked for the write, and is the key of
    /// the pair the sweep hands back.
    Write(String),
}

impl std::fmt::Display for HousekeepingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Profile(error) => write!(formatter, "the profile pass failed: {error}"),
            Self::Summary(error) => write!(formatter, "the summary call failed: {error}"),
            Self::Write(reason) => write!(formatter, "the record could not be written: {reason}"),
        }
    }
}

impl std::error::Error for HousekeepingError {}

impl From<WriteError> for HousekeepingError {
    fn from(error: WriteError) -> Self {
        Self::Write(error.to_string())
    }
}

/// The one call both background jobs make.
///
/// `purpose` is the source's `purpose=` argument, so the caller's logging and its engine
/// pool can tell a summary from a profile. The result is the parsed JSON *body*: the source
/// reads `choices[0].message.content` out of it and parses that again, and this crate keeps
/// that second half (see [`reply_content`]) because it is the part both prompts' shape
/// depends on.
pub trait BackgroundModel: Send + Sync {
    /// Ask for a JSON answer and hand back the body.
    ///
    /// # Errors
    ///
    /// [`ModelError::Transport`] when the call failed — the source's status-code branch,
    /// with the body snippet in the string — and [`ModelError::NoChoice`] when what came
    /// back has no `choices` in it at all. A reply that is present but is not JSON is *not*
    /// an error here: the body comes back as it arrived, because the two callers read an
    /// unreadable reply differently — one falls back, one raises.
    fn complete_json(&self, purpose: &str, messages: &[ChatMessage]) -> Result<Value, ModelError>;
}

/// `choices[0].message.content`, as a string ready to be parsed.
///
/// The source's four lines, including the `"{}"`: an empty `choices`, a choice with no
/// `message`, a message with no `content`, and a content that is `null` or `""` all answer
/// `"{}"`, which parses to an object with nothing in it and so reads as "the model said
/// nothing" in both callers.
///
/// A content that is not a string is the one place this is looser than the source, by one
/// step: Python renders it with `str()` — `"5"` for a number, `"None"` for a null — and
/// then fails to parse *that* as an object, which for the summarizer means the fallback and
/// for the profile worker means an error. Reading it as `"{}"` here reaches the same two
/// answers, because `"{}"` also parses to an object with no `summary` and no
/// `user_details`, and it does not depend on how a `Value` renders.
#[must_use]
pub fn reply_content(body: &Value) -> String {
    body.get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map_or_else(|| "{}".to_owned(), ToOwned::to_owned)
}

/// The record at `path`, or `None` — and `None` means leave the file alone.
///
/// `load_conversation_record_strict` (`agent.py:4185-4200`), and the reason 4d's forgiving
/// reader is not enough: that one stands in the empty record for a file it cannot read,
/// which is right for a turn that has to answer the person either way and is a way to erase
/// a conversation when the caller is about to write. A housekeeper handed an empty record
/// would summarize nothing and store nothing over the top of whatever was there.
///
/// So every failure is a `None`: an unreadable file, a file that is not JSON, a document
/// that is not an object, and an object whose `messages` or `summaries` is absent or not an
/// array. The last is what `ConversationRecord`'s required fields already enforce, which is
/// why it is a parse error here rather than a second look at the document.
///
/// Two readings are stricter than the source, in the same direction, and both are written
/// down in `docs/PARITY.md` terms: a row that is not an object is skipped by the source's
/// per-loop `isinstance` filter and fails the parse here, so the file is left alone rather
/// than trimmed around a row nobody can read; and a record carrying `"user_details": null`
/// is *missing* here and *present* there, because `Option` has one absence where a dict has
/// two. Nothing writes that value, and the difference is a profile backfilled on a record
/// that had an explicit null.
#[must_use]
pub fn load_conversation_record_strict(path: &Path) -> Option<ConversationRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The character a record belongs to, for the summary's own framing.
///
/// `conversation_character_profile` (4390-4405). A record at the root belongs to no
/// character, and a folder whose outline is missing, unreadable, or missing either half
/// still holds a conversation worth summarizing — all of those are `None`, which is the
/// prompt without its closing sentence rather than a refusal to summarize.
///
/// Two fields rather than an outline type: the prompt reads `name` and `locale` and nothing
/// else, and a full outline here would be a second one for `shed-characters` to disagree
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterName {
    /// The character's display name.
    pub name: String,
    /// Its locale, as the outline spells it.
    pub locale: String,
}

/// The outline beside `path`, if there is one that names a character.
///
/// `memory_dir` is a parameter rather than a field because the check is `folder ==
/// MEMORY_DIR`: a record at the root belongs to no character, and that is a question about
/// where the root is, not about the file.
#[must_use]
pub fn character_profile(memory_dir: &Path, path: &Path) -> Option<CharacterName> {
    let folder = path.parent()?;
    if folder == memory_dir {
        return None;
    }
    let outline: Value = serde_json::from_str(
        &std::fs::read_to_string(folder.join(CHARACTER_OUTLINE_FILENAME)).ok()?,
    )
    .ok()?;
    let named = |key: &str| outline.get(key).and_then(Value::as_str).unwrap_or("");
    let (name, locale) = (named("name"), named("locale"));
    // Python's `not outline.get("name")` — absence, an empty string, and a non-string all
    // answer `None`, and nothing else is read off the value: a name with padding reaches the
    // prompt with its padding, because the prompt is compared byte for byte.
    if name.is_empty() || locale.is_empty() {
        return None;
    }
    Some(CharacterName {
        name: name.to_owned(),
        locale: locale.to_owned(),
    })
}

/// How a transcript line spells the speaker.
///
/// The source builds this list twice, in two different ways, and the port keeps both rather
/// than reconciling them:
///
/// * the summarizer reads `message.get("role", "user")`, so a row whose role is the empty
///   string is shown as `": what they said"`;
/// * the profile worker reads `message.get("role") or "user"`, so the same row is shown as
///   `"user: what they said"`.
///
/// Both strip the content and skip a row whose content is empty — a row with nothing in it
/// is not evidence that anybody spoke. The difference is one character of one line of one
/// prompt, which is exactly the kind of thing a shared helper would silently change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptStyle {
    /// `message.get("role", "user")` — the summarizer's reading.
    Summary,
    /// `message.get("role") or "user"` — the profile worker's reading.
    Profile,
}

/// The `"{role}: {content}"` lines, one per row worth showing.
fn transcript_lines(messages: &[ConversationMessage], style: TranscriptStyle) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| {
            let content = message.content.trim();
            if content.is_empty() {
                return None;
            }
            let role = match style {
                TranscriptStyle::Summary => message.role.as_str(),
                TranscriptStyle::Profile if !message.role.is_empty() => message.role.as_str(),
                TranscriptStyle::Profile => "user",
            };
            Some(format!("{role}: {content}"))
        })
        .collect()
}

/// `text[-budget:]`, by characters.
///
/// Counting first and skipping is the same answer as `chars().rev().take(n).collect()
/// .chars().rev()`, and it keeps the property the source depends on: the transcript always
/// *ends* where the conversation ended, so the newest row is never the one that falls off.
#[must_use]
fn truncate_chars(text: &str, budget: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(budget)).collect()
}

/// The summarizer's transcript: the last 12 000 characters of its own line style.
#[must_use]
pub fn summary_transcript(messages: &[ConversationMessage]) -> String {
    truncate_chars(
        &transcript_lines(messages, TranscriptStyle::Summary).join("\n"),
        TRANSCRIPT_CHAR_BUDGET,
    )
}

/// The profile worker's transcript: the last 12 000 characters of its own line style.
#[must_use]
pub fn profile_transcript(messages: &[ConversationMessage]) -> String {
    truncate_chars(
        &transcript_lines(messages, TranscriptStyle::Profile).join("\n"),
        TRANSCRIPT_CHAR_BUDGET,
    )
}

/// One short, vague line for the memory log.
///
/// `summarize_conversation_messages` (3931-4010). No rows, or rows with nothing in them,
/// answer `""` without asking the model — the source returns before it builds a prompt, so a
/// conversation with nothing to say never spends a call saying so.
///
/// A reply that could not be read answers the last two transcript lines, joined with `" ; "`
/// and cut to 240 characters: a model that answered in prose still leaves a trace, and the
/// cap is what stops that trace from being the whole transcript walking back into a list
/// that is read on every later turn.
///
/// # Errors
///
/// Only [`ModelError::Transport`] or [`ModelError::NoChoice`], which are the source's
/// `status_code >= 400` `RuntimeError`. They reach the caller on purpose: the pass that
/// asked for this summary has not dropped anything yet, and a summary that could not be had
/// is the reason not to drop it.
pub fn summarize_conversation_messages(
    messages: &[ConversationMessage],
    character: Option<&CharacterName>,
    model: &dyn BackgroundModel,
) -> Result<String, ModelError> {
    let lines = transcript_lines(messages, TranscriptStyle::Summary);
    let transcript = truncate_chars(&lines.join("\n"), TRANSCRIPT_CHAR_BUDGET);
    if transcript.is_empty() {
        return Ok(String::new());
    }

    let mut system = SUMMARY_SYSTEM.to_owned();
    if let Some(character) = character {
        system.push_str(&format!(
            "This memory belongs to the character {} ({}).",
            character.name, character.locale
        ));
    }
    let body = model.complete_json(
        SUMMARY_PURPOSE,
        &[
            ChatMessage::new("system", system),
            ChatMessage::new("user", transcript),
        ],
    )?;

    let summary = serde_json::from_str::<Value>(&reply_content(&body))
        .ok()
        .and_then(|parsed| match parsed.get("summary") {
            Some(Value::String(text)) => Some(text.trim().to_owned()),
            _ => None,
        })
        .unwrap_or_default();
    if !summary.is_empty() {
        return Ok(summary);
    }
    // `" ; ".join(transcript_lines[-2:])[:240]` — the *untruncated* lines, which is why
    // `lines` is kept rather than re-split from the transcript: a transcript cut to 12 000
    // characters may have cut a line in half, and the fallback is not where that half-line
    // gets written into a memory.
    let from = lines.len().saturating_sub(FALLBACK_LINE_COUNT);
    Ok(truncate_chars(
        &lines[from..].join(" ; "),
        FALLBACK_CHAR_BUDGET,
    ))
}

/// The current profile as the profile prompt shows it: sorted, and spaced like Python.
///
/// `json.dumps(current_details or {}, ensure_ascii=False, sort_keys=True)`, which is three
/// promises the stored file does not make. `UserDetails` keeps insertion order — that order
/// is what the record stores and what the person's facts are introduced in — so this is the
/// one place the profile is sorted, and it is sorted because the source sorts it: a prompt
/// whose text moves between two identical profiles is a prompt that cannot be cached.
///
/// The spacing is Python's rather than JSON's: `json.dumps` writes `{"a": "b", "c": "d"}`,
/// with a space after each separator, and `serde_json::to_string` writes `{"a":"b"}`. That
/// is two bytes per fact of prompt text, which is nothing to a reader and everything to a
/// byte-for-byte comparison, so the pairs are joined here with the separators the source
/// uses and each string is escaped by `serde_json` — which escapes the same characters the
/// same way, and leaves non-ASCII alone exactly as `ensure_ascii=False` asks.
#[must_use]
pub fn profile_prompt_json(details: &UserDetails) -> String {
    let sorted: BTreeMap<&str, &str> = details
        .pairs()
        .iter()
        .map(|(label, text)| (label.as_str(), text.as_str()))
        .collect();
    let body = sorted
        .iter()
        .map(|(label, text)| {
            let label = serde_json::to_string(label).unwrap_or_else(|_| "\"\"".to_owned());
            let text = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned());
            format!("{label}: {text}")
        })
        .collect::<Vec<String>>()
        .join(", ");
    format!("{{{body}}}")
}

/// Fold what the person just said into their profile.
///
/// `update_conversation_user_details` (4013-4080). The answer is the whole profile and not a
/// patch: the model is shown the profile as it stands and answers with the updated one, so a
/// fact it was not reminded of is a fact that stays. This function does not merge the answer
/// into what it was given — the source does not either, and a merge here would resurrect a
/// fact the model was asked to drop. What it does apply is `normalize_user_details`, because
/// the answer arrives as JSON and every other profile in this crate was read through that
/// function on its way in.
///
/// An empty transcript is the current profile, unchanged, and no call is made.
///
/// # Errors
///
/// [`ModelError::NotJson`] for a reply that did not parse, or that parsed to something with
/// no `user_details` **object** in it — the source's two `RuntimeError`s. Both raise rather
/// than answering with the old profile because the caller marks these rows as read the
/// moment this returns, and a silent success would retire what the person said on the
/// strength of a reply that never carried it.
pub fn update_conversation_user_details(
    messages: &[ConversationMessage],
    current: &UserDetails,
    model: &dyn BackgroundModel,
) -> Result<UserDetails, ModelError> {
    let transcript = profile_transcript(messages);
    if transcript.is_empty() {
        return Ok(current.clone());
    }
    let body = model.complete_json(
        PROFILE_PURPOSE,
        &[
            ChatMessage::new("system", PROFILE_SYSTEM),
            ChatMessage::new(
                "user",
                format!(
                    "Current profile:\n{}\n\nNew messages:\n{transcript}",
                    profile_prompt_json(current)
                ),
            ),
        ],
    )?;

    let parsed: Value =
        serde_json::from_str(&reply_content(&body)).map_err(|_| ModelError::NotJson)?;
    match parsed.get("user_details") {
        Some(details @ Value::Object(_)) => Ok(normalize_user_details(details)),
        // A `user_details` that is a string, a list, a number or a null is the source's
        // `if not isinstance(details, dict)`: the reply answered the wrong question.
        _ => Err(ModelError::NotJson),
    }
}

/// What the profile pass decided about one record, before it asked anybody anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileWork {
    /// What there is to do.
    pub kind: ProfileWorkKind,
    /// The rows the profile has not read — the whole transcript when there is no profile to
    /// have read anything, because a marker cannot speak for a section that is not there.
    pub messages: Vec<ConversationMessage>,
    /// The profile as it stands.
    pub current: UserDetails,
    /// The newest of *those* rows: the marker this pass would move the profile to.
    ///
    /// The newest row the model actually saw, not the newest row in the file, which is what
    /// makes "a turn landed during the call" a pass that gets read next time rather than a
    /// turn that is never read at all.
    pub covered_through: Option<String>,
}

/// The three things a profile pass can decide to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileWorkKind {
    /// Ask the model, then store what it answered.
    Update,
    /// The record has no profile at all and there is nothing to ask about: store an empty one
    /// anyway, because its absence is what the next visit would read as a stranger's.
    WriteEmpty,
    /// Nothing to do, and nothing to write.
    None,
}

/// Decide what the profile pass would do, from a record already read.
///
/// The front half of `update_conversation_profile` (4202-4256), split out because it is the
/// half with rules in it: the marker decides which rows count, a tail of reply beats alone
/// teaches nothing about the person, and a record with no profile is read from its whole
/// transcript.
#[must_use]
pub fn profile_work_for(record: &ConversationRecord) -> ProfileWork {
    let profile_missing = record.user_details.is_none();
    let messages = if profile_missing {
        record.messages.clone()
    } else {
        unprofiled_messages(
            &record.messages,
            record.user_details_covered_through.as_deref(),
        )
    };
    let covered_through = newest_message_timestamp(&messages)
        .filter(|stamp| !stamp.is_empty())
        .map(ToOwned::to_owned);
    // Assistant beats alone teach nothing about the person, but they are context for the next
    // user message, so they stay unread rather than being marked covered. The test for one is
    // the source's `str(item.get("role") or "") == "user"`: no strip, and an empty role is
    // not a user.
    let has_work = !messages.is_empty()
        && messages
            .iter()
            .any(|message| message.role.as_str() == "user");
    let kind = if has_work {
        ProfileWorkKind::Update
    } else if profile_missing {
        ProfileWorkKind::WriteEmpty
    } else {
        ProfileWorkKind::None
    };
    ProfileWork {
        kind,
        messages,
        current: record.user_details.clone().unwrap_or_default(),
        covered_through,
    }
}

/// Fold every row the profile has not read into the record, now.
///
/// `update_conversation_profile` (4202-4256), called as each turn ends and again by the idle
/// pass for a record left alone since before the profile existed.
///
/// The source wraps the decision in `CONVERSATION_LOCK`, asks the model outside it, then
/// takes the lock again to reload and write. There is no such lock here — this crate owns no
/// shared state to lock — so the shape survives as the *order*: read, decide, ask, read
/// again, write. The reload is the part that is not ceremony. A turn that appended while the
/// model was answering would be overwritten by a write built from the earlier read, and the
/// person's own words are the one thing this file is not allowed to lose.
///
/// Returns whether the file was written. A profile already up to date is not rewritten.
///
/// # Errors
///
/// [`HousekeepingError::Profile`] when the call failed or answered with something that was
/// not a profile, and [`HousekeepingError::Write`] when the record could not be stored. The
/// idle pass treats either as "stop before dropping anything"; the worker swallows it,
/// because the reply has already been given and there is nobody left to tell.
pub fn update_conversation_profile(
    path: &Path,
    model: &dyn BackgroundModel,
) -> Result<bool, HousekeepingError> {
    let Some(record) = load_conversation_record_strict(path) else {
        return Ok(false);
    };
    let work = profile_work_for(&record);
    if work.kind == ProfileWorkKind::None {
        return Ok(false);
    }

    let details = match work.kind {
        ProfileWorkKind::Update => {
            update_conversation_user_details(&work.messages, &work.current, model)
                .map_err(HousekeepingError::Profile)?
        }
        // `write_empty_profile`: the answer is `{}` and the model is never asked, so a record
        // from before the profile existed stops being one even when the person has so far
        // only been answered to.
        ProfileWorkKind::WriteEmpty | ProfileWorkKind::None => UserDetails::default(),
    };

    let Some(record) = load_conversation_record_strict(path) else {
        return Ok(false);
    };
    // The marker moves only when this pass saw a row it could name; a record whose rows all
    // have unreadable stamps keeps whatever marker it had rather than getting one that points
    // at nothing. That is the source's `if covered_through:` over a `dict(record)`.
    let marker = work
        .covered_through
        .or_else(|| record.user_details_covered_through.clone());
    let updated = ConversationRecord {
        user_details: Some(details),
        user_details_covered_through: marker,
        ..record
    };
    write_conversation_record_file(path, &updated)?;
    Ok(true)
}

/// Which conversations have a profile worker running.
///
/// `CONVERSATION_PROFILE_WORKERS` and its lock (4210, 4289-4310). One worker per
/// conversation rather than a queue: a worker already running will see the new rows on its
/// next pass, so a second thread would only wait for the same lock — and with one resident
/// model, would then ask it the same question twice.
///
/// The set is keyed by path, which is what the source keys it by and is also the only thing
/// that tells two characters' `client-x.json` files apart.
#[derive(Debug, Default)]
pub struct ProfileWorkers {
    active: Arc<Mutex<HashSet<PathBuf>>>,
}

impl ProfileWorkers {
    /// No workers running.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `path` has a worker on it right now.
    ///
    /// A poisoned lock reads as "nothing is running". The source's lock has no poisoned state
    /// to read, and a worker that panicked while holding the set must not make every other
    /// conversation look busy forever — the panic is the bug, and this is only the answer the
    /// next caller gets.
    #[must_use]
    pub fn is_active(&self, path: &Path) -> bool {
        self.lock().contains(path)
    }

    /// How many conversations are being profiled.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing is being profiled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Profile `path` in the background, or do nothing if it is already being profiled.
    ///
    /// `start_conversation_profile_update` (4289-4310). The claim happens here rather than in
    /// the thread, which is the difference between "at most one worker per conversation" and
    /// "usually one": claiming inside the thread would let two callers both start one and let
    /// whichever got there first win.
    ///
    /// Whether a worker was started. A `false` is either "one is already running" or "no
    /// thread could be started", and the caller does not need to know which: the source
    /// re-raises the second, and its only caller swallows it (decision 10).
    #[must_use]
    pub fn start(&self, path: &Path, model: Arc<dyn BackgroundModel>) -> bool {
        {
            let mut active = self.lock();
            if active.contains(path) {
                return false;
            }
            active.insert(path.to_path_buf());
        }
        let claimed = path.to_path_buf();
        let guard = ClaimGuard {
            active: Arc::clone(&self.active),
            path: claimed.clone(),
        };
        let started = std::thread::Builder::new()
            .name("conversation-profile".to_owned())
            .spawn(move || {
                profile_conversation_worker(&claimed, model.as_ref());
                // The guard is dropped here, or during an unwind: either way the
                // conversation goes back, which is the source's `finally`.
                drop(guard);
            });
        match started {
            Ok(_joined) => true,
            Err(_) => {
                // The source discards and re-raises. There is nothing to re-raise into, and
                // everything to give back.
                self.lock().remove(path);
                false
            }
        }
    }

    /// Wait for in-flight profile saves, up to `timeout`. Whether they all finished.
    ///
    /// `wait_for_conversation_profile_workers` (4338-4349). A turn never waits on this: it
    /// exists for an orderly shutdown, and for a test that is about to delete the folder a
    /// worker is still writing into.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.lock().is_empty() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The set, with a poisoned lock repaired on the way in.
    fn lock(&self) -> MutexGuard<'_, HashSet<PathBuf>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for ProfileWorkers {
    /// The source has no equivalent and needs one: its workers are daemon threads, so a
    /// process that exits mid-summary leaves one half-written. Dropping the value that owns
    /// the workers is the last moment they can still be waited for, and five seconds is the
    /// source's own shutdown timeout.
    fn drop(&mut self) {
        let _finished = self.wait(Duration::from_secs(5));
    }
}

/// Gives a conversation back to whoever wants it next, however the worker ended.
///
/// A `finally` in a type, because the alternative is a `match` on the result of every call
/// in the thread body, and the one case that matters — a panic inside a caller's `BackgroundModel`
/// implementation — is the case a `match` cannot see coming.
struct ClaimGuard {
    active: Arc<Mutex<HashSet<PathBuf>>>,
    path: PathBuf,
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.path);
    }
}

/// Keep profiling one conversation until the record holds nothing new.
///
/// `profile_conversation_worker` (4261-4287), less the logging: the layer map does not allow
/// `shed-logging`, so a failure is swallowed rather than warned about, which is the behaviour
/// the callers and the tests care about. The reason it loops rather than running once is the
/// reason the whole feature exists: turns land while the model is answering, and looping is
/// what makes "saved as the turn ends" true for the last turn as well as the first.
///
/// The loop follows the marker rather than simply repeating. A row whose timestamp cannot be
/// read is unread forever — `unprofiled_messages` keeps a row it cannot place — so a loop
/// that only asked "did we write" would question the model about the same row until the
/// process stopped. Comparing the marker makes that cost one retry instead.
pub fn profile_conversation_worker(path: &Path, model: &dyn BackgroundModel) {
    let mut last_marker: Option<String> = None;
    loop {
        match update_conversation_profile(path, model) {
            Ok(true) => {}
            // `false` is "nothing left to do" and an error is "housekeeping will try again":
            // neither is a reason to keep asking.
            Ok(false) | Err(_) => return,
        }
        let marker = load_conversation_record_strict(path)
            .and_then(|record| record.user_details_covered_through)
            .unwrap_or_default();
        if last_marker.as_deref() == Some(marker.as_str()) {
            return;
        }
        last_marker = Some(marker);
    }
}

/// Save the turn's profile, and never let that cost the turn.
///
/// `schedule_conversation_profile_update` (4313-4326). The reply has already been given by
/// the time this runs, so nothing here — a key the filename builder refuses, a folder it
/// cannot make, a thread the process will not start — may reach the caller. That is why this
/// returns nothing: a caller handed an `Option` would be handed back a decision it has
/// already been told not to make.
///
/// The key is resolved through [`conversation_filename`] rather than joined to the root,
/// because a legacy session-keyed record is a key that names nothing, and profiling it would
/// be background work on words nobody reads.
pub fn schedule_conversation_profile_update(
    workers: &ProfileWorkers,
    root: &ConversationRoot,
    conversation_key: &str,
    character_slug: Option<&str>,
    model: Arc<dyn BackgroundModel>,
) {
    let Ok(filename) = conversation_filename(conversation_key) else {
        return;
    };
    let path = root.record_path(&filename, character_slug);
    let _started = workers.start(&path, model);
}

/// The compaction and timing settings the sweep runs with.
///
/// The source reads seven module constants. Here they are one value the caller resolved,
/// because a sweep that read them itself would need `shed-settings` — a crate below this one
/// in the layer map, whose job is naming keys rather than deciding what a sweep does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HousekeepingSettings {
    /// How recent a row has to be to be kept exactly, in days.
    pub detailed_days: f64,
    /// How far back a summary may reach, in days. Zero disables the age cut.
    pub summary_days: f64,
    /// The exact transcript's cap, in messages.
    pub max_messages: i64,
    /// The summary list's cap, which the source hard-codes as `summaries[-36:]` inside the
    /// pass and which is `max_summaries` here so the plan and the write cannot be told
    /// different numbers.
    pub max_summaries: usize,
    /// How long a file must have been untouched before this pass reads it. A conversation
    /// being talked in is not a conversation to tidy.
    pub idle: Duration,
    /// How long a failed record is left alone. A model that is down should not be asked every
    /// poll.
    pub cooldown: Duration,
    /// How often the loop wakes. Carried here so the settings value and the thread's sleep
    /// cannot disagree, and passed to [`spawn`] as well for a caller that would rather say it
    /// out loud.
    pub poll: Duration,
}

impl Default for HousekeepingSettings {
    /// The app's own defaults, so a caller with no settings loaded sweeps the way the app
    /// does. The reference suite's fixtures override most of these — detailed 7 days, no age
    /// cut, a cap of ten messages — and pass them explicitly rather than editing this.
    fn default() -> Self {
        Self {
            detailed_days: 7.0,
            summary_days: 0.0,
            max_messages: 240,
            max_summaries: 36,
            idle: Duration::from_secs(300),
            cooldown: Duration::from_secs(900),
            poll: Duration::from_secs(30),
        }
    }
}

/// What the sweep remembers between runs.
///
/// `CONVERSATION_HOUSEKEEPING_CHECKED_MTIME` and `CONVERSATION_HOUSEKEEPING_RETRY_AT`
/// (4189-4201), as a value rather than as module state. Both are keyed by path, and both are
/// wall-clock rather than monotonic, because the first is a file's modified time — which is
/// itself wall-clock, and comparing it against a monotonic instant would be comparing two
/// unrelated clocks.
///
/// In memory only, as the source's comment says: a restart is as good a moment to retry as
/// any.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HousekeepingState {
    /// The modified time each record was last examined at. A file that needed nothing is not
    /// read again until something appends to it and the mtime moves.
    pub checked: HashMap<PathBuf, SystemTime>,
    /// The instant before which a failed record is left alone.
    pub retry_at: HashMap<PathBuf, SystemTime>,
}

impl HousekeepingState {
    /// Forget everything: every record is unswept, and nothing is cooling down.
    pub fn clear(&mut self) {
        self.checked.clear();
        self.retry_at.clear();
    }
}

/// One conversation the sweep could not finish, and which half said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepFailure {
    /// The profile pass failed. The record was left alone, and nothing was dropped.
    Profile(ModelError),
    /// The summary call failed. The record was left alone.
    Summary(ModelError),
    /// The trimmed record could not be stored. The old one is still on disk.
    Write(String),
}

impl std::fmt::Display for SweepFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Profile(error) => write!(formatter, "the profile pass failed: {error}"),
            Self::Summary(error) => write!(formatter, "the summary call failed: {error}"),
            Self::Write(reason) => write!(formatter, "the record could not be written: {reason}"),
        }
    }
}

impl std::error::Error for SweepFailure {}

impl From<HousekeepingError> for SweepFailure {
    fn from(error: HousekeepingError) -> Self {
        match error {
            HousekeepingError::Profile(error) => Self::Profile(error),
            HousekeepingError::Summary(error) => Self::Summary(error),
            HousekeepingError::Write(reason) => Self::Write(reason),
        }
    }
}

/// Trim and summarize one idle conversation, if it needs anything.
///
/// `housekeep_conversation_record_file` (4408-4478). Three things happen, in this order, and
/// the order is the point:
///
/// 1. the profile is brought up to date, and a failure there stops the pass *before* any row
///    is dropped — the rows it would have read are the only copy of what the person said;
/// 2. the plan is built from the record as it now stands, and a plan that changes nothing is
///    a write that does not happen;
/// 3. the summary is asked for with no lock held, and the record is then read *again* and the
///    first `consumed_count` rows dropped from **that** rather than from the record the plan
///    read. The transcript is append-only, so a turn that landed while the model was being
///    asked sits after the consumed prefix and survives it.
///
/// A record that cannot be read is `Ok(())` with nothing done, which is what "leave it
/// alone" means when there is nothing to leave — and which is why a damaged record is not a
/// [`SweepFailure`] and gets no cooldown: the next sweep will read it again, find it damaged
/// again, and leave it alone again, at the cost of a read rather than of a conversation.
///
/// # Errors
///
/// Whatever the profile pass or the summary call reports, plus a record that could not be
/// stored.
pub fn housekeep_conversation_record_file(
    root: &ConversationRoot,
    path: &Path,
    now: DateTime<Utc>,
    settings: &HousekeepingSettings,
    model: &dyn BackgroundModel,
) -> Result<(), HousekeepingError> {
    update_conversation_profile(path, model)?;

    let Some(record) = load_conversation_record_strict(path) else {
        return Ok(());
    };
    let plan = plan_conversation_compaction(
        &record,
        now,
        settings.detailed_days,
        settings.summary_days,
        settings.max_messages,
        settings.max_summaries,
    );
    if plan.consumed_count == 0
        && plan.messages_to_summarize.is_empty()
        && plan.retained_summaries == record.summaries
    {
        return Ok(());
    }

    let summary_text = if plan.messages_to_summarize.is_empty() {
        String::new()
    } else {
        summarize_conversation_messages(
            &plan.messages_to_summarize,
            character_profile(&root.memory_dir, path).as_ref(),
            model,
        )
        .map_err(HousekeepingError::Summary)?
    };
    write_trimmed_record(path, &plan, &summary_text, settings.max_summaries)
}

/// Drop the rows the plan read, and store what is left beside everything else in the file.
///
/// The second read is why this is a function of its own rather than the tail of the pass
/// above: a model call sits between the plan and this write, and a person who sent another
/// message during it is not one of the rows the plan read. The transcript is append-only, so
/// dropping `consumed_count` rows from the *newest* read keeps their turn; dropping them from
/// the record the plan read would not.
///
/// A file that now holds **fewer** rows than the plan consumed is not the same file any more:
/// something removed it and a turn rebuilt it. Its rows are gone either way, and the new
/// conversation is the only thing left worth protecting, so this one is left alone.
///
/// The write starts from the record as it now stands rather than from a record-shaped value:
/// the delivery style, the profile, and anything else beside the two lists is part of the
/// conversation, and a rewrite must not be how it is lost.
///
/// # Errors
///
/// [`HousekeepingError::Write`] when the trimmed record could not be stored. The untrimmed one
/// is still on disk, which is why a trim that did not land is worth a cooldown: the same rows
/// will be summarized again, and a second summary of the same words is the one thing the list
/// cannot be trusted to notice.
fn write_trimmed_record(
    path: &Path,
    plan: &ConversationCompactionPlan,
    summary_text: &str,
    max_summaries: usize,
) -> Result<(), HousekeepingError> {
    let Some(record) = load_conversation_record_strict(path) else {
        return Ok(());
    };
    if record.messages.len() < plan.consumed_count {
        return Ok(());
    }
    let mut summaries = plan.retained_summaries.clone();
    if !summary_text.is_empty() {
        let rows = &plan.messages_to_summarize;
        summaries.push(ConversationSummary {
            start_timestamp: rows[0].timestamp.clone(),
            end_timestamp: rows[rows.len() - 1].timestamp.clone(),
            summary: summary_text.to_owned(),
        });
    }
    // Oldest block first, so the list reads as the conversation did. Comparing the stored
    // strings rather than the instants is the source's `key=lambda item: str(...) or ""`: two
    // blocks whose stamps cannot be read keep the order they arrived in, because equal keys
    // sort stably and an empty stamp equals another empty stamp.
    summaries.sort_by(|left, right| left.end_timestamp.cmp(&right.end_timestamp));
    // `summaries[-max_summaries:]`, quirk included: `[-0:]` is the whole list rather than the
    // empty one, and `len - 0` is `len`, which would delete a record's entire memory on the
    // pass after an operator unset the cap. `compaction.rs`'s `newest_kept` answers zero the
    // same way — two answers for "no cap" in one crate is how a list stops being predictable.
    let kept = if max_summaries == 0 {
        summaries
    } else {
        let from = summaries.len().saturating_sub(max_summaries);
        summaries.split_off(from)
    };
    let updated = ConversationRecord {
        messages: record.messages[plan.consumed_count..].to_vec(),
        summaries: kept,
        ..record
    };
    write_conversation_record_file(path, &updated)?;
    Ok(())
}

/// One sweep over every idle conversation that has changed since the last.
///
/// `run_conversation_housekeeping` (4481-4543). Files are examined once per modified time,
/// which is what lets a loop that wakes every thirty seconds still read an untouched
/// conversation once an hour; the cooldown entries are what a failed pass leaves behind.
///
/// `active_chat_requests` is the source's global counter, taken as a number: the summarizer
/// shares the resident model with live turns, so a sweep that runs while somebody is being
/// answered returns before it reads a file. A caller with no such counter passes zero and
/// gets a sweep that always runs.
///
/// The state is pruned at the end of the walk, which is the difference between a long-lived
/// server and one that remembers every conversation it ever had a view of: a record that was
/// deleted, or moved into a character folder, or named for a session nobody uses, leaves both
/// maps so they stay the size of the store rather than the size of the uptime.
///
/// Returns what failed, as `(path, why)` in the order the files were walked. Nothing
/// propagates: every failure is one entry and one cooldown, and the sweep carries on to the
/// next file, which is the source's `except Exception` around the whole body. A root that
/// cannot be listed at all is not a failure either — it is an empty sweep, because there is
/// nothing here to report a path against.
pub fn run_housekeeping(
    root: &ConversationRoot,
    settings: &HousekeepingSettings,
    state: &mut HousekeepingState,
    active_chat_requests: usize,
    now: DateTime<Utc>,
    model: &dyn BackgroundModel,
) -> Vec<(PathBuf, SweepFailure)> {
    let mut failures = Vec::new();
    if active_chat_requests > 0 {
        return failures;
    }
    let Ok(files) = root.record_files() else {
        return failures;
    };
    let now_at = to_system_time(now);

    let mut seen: HashSet<PathBuf> = HashSet::new();
    for path in files {
        if !names_a_conversation(path.file_name().and_then(|name| name.to_str())) {
            // A legacy session-keyed record is read by nothing now, and summarizing it would
            // cost a model call to preserve words nobody looks at. The character outline never
            // reaches here — `record_files` skips it by name.
            continue;
        }
        seen.insert(path.clone());
        let Some(modified) = idle_since(&path, state, settings, now_at) else {
            continue;
        };
        if let Some(failure) = sweep_one_file(root, &path, modified, now, settings, model, state) {
            failures.push((path, failure));
        }
    }

    state.checked.retain(|path, _| seen.contains(path));
    state.retry_at.retain(|path, _| seen.contains(path));
    failures
}

/// The modified time of a record this sweep has not examined yet, or `None` for "not due".
///
/// The four reasons a file is passed over, in the source's order: its metadata cannot be
/// read, it has not been touched since it was last examined, it is cooling down from a failed
/// pass, and it has not been quiet long enough. The first of them is the one that makes the
/// sweep cheap — a conversation nobody has written to is skipped on a `HashMap` lookup rather
/// than on a model call — and it is why the map holds the mtime rather than a timestamp.
///
/// A file stamped ahead of the clock — a network share, a clock that was wrong once — has no
/// `Duration` between then and now, and that reads as "not idle yet", which is what the
/// source's comparison answered for the same file.
fn idle_since(
    path: &Path,
    state: &HousekeepingState,
    settings: &HousekeepingSettings,
    now_at: SystemTime,
) -> Option<SystemTime> {
    let modified = path.metadata().and_then(|meta| meta.modified()).ok()?;
    if state.checked.get(path) == Some(&modified) {
        return None;
    }
    if state
        .retry_at
        .get(path)
        .is_some_and(|earliest| *earliest > now_at)
    {
        return None;
    }
    if now_at
        .duration_since(modified)
        .is_ok_and(|quiet| quiet < settings.idle)
    {
        return None;
    }
    Some(modified)
}

/// One file's pass, and what it left in `state` either way.
///
/// Returns the failure the caller reports, so the cooldown and the reported reason cannot be
/// two different decisions made in two different places.
fn sweep_one_file(
    root: &ConversationRoot,
    path: &Path,
    modified: SystemTime,
    now: DateTime<Utc>,
    settings: &HousekeepingSettings,
    model: &dyn BackgroundModel,
    state: &mut HousekeepingState,
) -> Option<SweepFailure> {
    match housekeep_conversation_record_file(root, path, now, settings, model) {
        Ok(()) => {
            // Re-stat rather than reuse: the pass may have rewritten the file, and recording
            // the *old* mtime would mean reading it again on the next poll. A file that went
            // away mid-pass keeps the mtime it had, which is the mtime it had.
            let examined = path
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(modified);
            state.checked.insert(path.to_path_buf(), examined);
            state.retry_at.remove(path);
            None
        }
        Err(error) => {
            let cooldown = ChronoDuration::from_std(settings.cooldown).unwrap_or_default();
            state
                .retry_at
                .insert(path.to_path_buf(), to_system_time(now + cooldown));
            Some(error.into())
        }
    }
}

/// The housekeeping loop, on a thread of its own.
///
/// `conversation_housekeeping_loop` and `start_conversation_housekeeper` (4546-4560), which
/// the brief asks for as a `spawn` that takes the poll interval rather than as a `while True`
/// inside a binary. Nothing starts here until this is called.
///
/// The returned handle is what stops it: dropping it asks for one more sweep to finish and
/// then ends the loop. A sweep in progress is not interrupted — it is a model call, and a
/// call abandoned halfway is a record half-trimmed — so a shutdown that wants to be quick
/// wants a short `poll`, not a kill.
///
/// # Errors
///
/// The thread could not be started. The source cannot report this at all: its
/// `Thread.start()` raises into whatever was starting the app.
pub fn spawn(
    root: ConversationRoot,
    settings: HousekeepingSettings,
    model: Arc<dyn BackgroundModel>,
    active_chat_requests: Arc<AtomicUsize>,
    poll: Duration,
) -> io::Result<Housekeeper> {
    let stopping = Arc::new(AtomicBool::new(false));
    let thread = {
        let stopping = Arc::clone(&stopping);
        std::thread::Builder::new()
            .name("conversation-housekeeper".to_owned())
            .spawn(move || {
                let mut state = HousekeepingState::default();
                while !stopping.load(Ordering::Acquire) {
                    let live = active_chat_requests.load(Ordering::Acquire);
                    let _failures = run_housekeeping(
                        &root,
                        &settings,
                        &mut state,
                        live,
                        utc_now(),
                        model.as_ref(),
                    );
                    // The source sleeps in one call. Sleeping in slices is what makes the
                    // stop mean something: a poll of thirty seconds would otherwise hold a
                    // shutdown for half a minute for no reason anybody asked for.
                    let mut slept = Duration::ZERO;
                    while !stopping.load(Ordering::Acquire) && slept < poll {
                        let step = Duration::from_millis(50).min(poll - slept);
                        std::thread::sleep(step);
                        slept += step;
                    }
                }
            })?
    };
    Ok(Housekeeper {
        stopping,
        thread: Some(thread),
    })
}

/// The running housekeeping loop, and the way to stop it.
#[derive(Debug)]
pub struct Housekeeper {
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Housekeeper {
    /// Ask the loop to stop after its current sweep, and wait for it.
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // A thread that panicked has no loop left to join, and its absence is not a
            // failure the caller can act on: the next sweep would have found the same broken
            // state anyway.
            let _joined = thread.join();
        }
    }
}

impl Drop for Housekeeper {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A `SystemTime` for the sweep's clock.
///
/// The sweep compares a `DateTime<Utc>` against file mtimes, which are `SystemTime`s, and the
/// source does the same thing by reducing both to `time.time()`. Before the epoch there is no
/// `SystemTime`, and the answer is the epoch itself: a now from 1970 is older than every file,
/// so nothing is idle and nothing is swept, which is the safe direction for a clock that has
/// lost its settings.
///
/// The sub-second part is carried across rather than dropped. A test that sweeps at
/// `mtime + idle` to the second is the common case, and a `now` truncated *down* to the
/// second would answer "not idle yet" on the very boundary the test is pinning.
fn to_system_time(now: DateTime<Utc>) -> SystemTime {
    let seconds = now.timestamp().max(0) as u64;
    UNIX_EPOCH
        + Duration::from_secs(seconds)
        + Duration::from_nanos(u64::from(now.timestamp_subsec_nanos()))
}

/// The sweep's own clock, for the loop that has no caller to borrow one from.
///
/// The workspace builds chrono without its `clock` feature, so `Utc::now()` does not exist
/// anywhere in this crate — which is the point: every rule here takes its moment as a
/// parameter, and a library that could read the clock is a library whose tests quietly stop
/// being reproducible. The loop is the one place that genuinely wants "now", so it reads the
/// wall clock from `std` and turns it into the datetime the compaction plan wants. A clock
/// set before 1970 answers the epoch, which is the same nothing-is-idle answer
/// [`to_system_time`] gives from the other side.
fn utc_now() -> DateTime<Utc> {
    let Ok(since_epoch) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return DateTime::UNIX_EPOCH;
    };
    let seconds = i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX);
    DateTime::from_timestamp(seconds, since_epoch.subsec_nanos()).unwrap_or(DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::USER_DETAILS_OTHER_MAX_CHARS;

    /// The prompt's promise about `other` and the loader's cap on it are one number.
    ///
    /// The prompt cannot import the constant, so this is the only thing standing between the
    /// two: change `USER_DETAILS_OTHER_MAX_CHARS` without changing the sentence and a model
    /// will keep being invited to write a profile field the next load silently cuts.
    #[test]
    fn the_profile_prompt_promises_the_cap_the_loader_applies() {
        let promise = format!("{USER_DETAILS_OTHER_MAX_CHARS} characters");
        assert!(
            PROFILE_SYSTEM.contains(&promise),
            "the prompt must spell out the cap by the same number the code applies: it says \
             {:?}, which carries no {promise}",
            PROFILE_SYSTEM
        );
    }

    #[test]
    fn a_transcript_keeps_its_newest_line_when_it_is_cut() {
        let rows: Vec<ConversationMessage> = (0..40)
            .map(|index| ConversationMessage {
                role: "user".to_owned(),
                content: "x".repeat(500) + &index.to_string(),
                timestamp: String::new(),
            })
            .collect();
        let transcript = profile_transcript(&rows);
        assert_eq!(transcript.chars().count(), TRANSCRIPT_CHAR_BUDGET);
        let newest = format!("user: {}39", "x".repeat(500));
        assert!(
            transcript.ends_with(&newest),
            "a cut transcript must end where the conversation ended"
        );
    }

    #[test]
    fn the_two_styles_disagree_about_an_empty_role() {
        let rows = vec![ConversationMessage {
            role: String::new(),
            content: "hello".to_owned(),
            timestamp: String::new(),
        }];
        assert_eq!(summary_transcript(&rows), ": hello");
        assert_eq!(profile_transcript(&rows), "user: hello");
    }

    #[test]
    fn the_profile_prompt_is_sorted_and_spaced_like_python() {
        let details = UserDetails::new(vec![
            ("zeta".to_owned(), "one".to_owned()),
            ("alpha".to_owned(), "two".to_owned()),
        ]);
        assert_eq!(
            profile_prompt_json(&details),
            r#"{"alpha": "two", "zeta": "one"}"#
        );
        assert_eq!(profile_prompt_json(&UserDetails::default()), "{}");
    }

    #[test]
    fn an_absent_or_empty_content_reads_as_the_model_saying_nothing() {
        for body in [
            serde_json::json!({}),
            serde_json::json!({"choices": []}),
            serde_json::json!({"choices": [{"message": {}}]}),
            serde_json::json!({"choices": [{"message": {"content": "   "}}]}),
            serde_json::json!({"choices": [{"message": {"content": null}}]}),
        ] {
            assert_eq!(reply_content(&body), "{}", "{body}");
        }
    }
}
