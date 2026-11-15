//! One conversation record as it lives on disk: where it is, how it reads back,
//! and how it is written.
//!
//! Ported from `agent.py:3785-3930` — `conversation_record_path`,
//! `conversation_record_for_key`, `read_conversation_record_file`,
//! `load_conversation_record`, `conversation_delivery_style`,
//! `write_conversation_record_file`, `save_conversation_record`. This is
//! `shed_store::atomic_write_json`'s first production caller, which is the reason
//! the layer map (`shed-gates`'s `LAYERS`) allows `shed-conversation → shed-store`.
//!
//! ## The root is a value, and so is the character's folder
//!
//! `MEMORY_DIR` is a module global in the source, computed once at import. Here it
//! is a field of [`ConversationRoot`], so a test can point a conversation at a
//! temporary folder and the app can point it at whatever `shed-config` resolves to.
//! The character's folder arrives the same way — as a slug, not as a profile dict:
//! this crate's layer position does not include `shed-characters`, and the caller
//! that holds the profile already knows the folder's name.
//!
//! ## The filename is computed by the caller, once
//!
//! [`crate::conversation_filename`] is the door that refuses a key which names
//! neither an address nor a browser, and a refusal has to stay a refusal at the
//! boundary rather than become a filename three layers down. So every function here
//! takes the *validated* name and cannot fail on it. A caller that skipped that step
//! would be joining unvalidated text into a path, which is the one thing
//! `filename.rs` exists to prevent.
//!
//! ## Reading is forgiving, writing is not
//!
//! A record that cannot be read reads as the empty record — [`ConversationRecord::default()`] —
//! and not as an error, which is the source's own choice and its stated reason: the
//! turn reads these inside its lock and the slow housekeeping loop reads one with
//! nobody watching, and neither has anything better to do about a damaged file than
//! carry on without it. An empty record is the honest reading of a conversation
//! nobody can read. A caller handed a `Result` would only ever return the same empty
//! record, so the choice is made once here instead of at every call site.
//!
//! Writing is the opposite: [`write_conversation_record_file`] returns
//! [`WriteError`], because a transcript that cannot be stored is a turn's work lost
//! and the caller is the only place that can say so.
//!
//! ## No `.bak`, on purpose
//!
//! `keep_backup=False` at the source's call site is [`Backup::Skip`] here: a
//! transcript is rewritten every turn, and a second copy of it beside each record is
//! space spent preserving words that are already in the file they were copied from.
//! A mind-state record keeps its backup (Stage 7 recovers from it); this file does
//! not, and the test that pins it is in `shed-store`'s `tests/document_writer.rs`.
//!
//! ## Two writers, one file
//!
//! The turn and the profile worker both write this file, which is the whole reason
//! the write goes through a temp file and a rename. A reader that caught a half-write
//! would read a truncated conversation, and a truncated conversation is the person's
//! memory with the end of it missing.
//!
//! ## The one move: an address-keyed record taken by a browser
//!
//! Before conversations were keyed by browser, a record was named for the address.
//! [`claim_legacy_conversation`] hands such a record to the first browser of that
//! address to come back after the change, rather than letting it watch a person it
//! already knows start a new history. Only a browser that is not known to share the
//! address can take it: once two ids have been recorded there there is no
//! deterministic answer to whose words those were, so neither takes them. The client
//! records it checks are a parameter, not a directory read — the client-file store is
//! Stage 7's — and the guard order is the source's, because a target that already
//! exists is the one case where the answer must not be "the newest wins".

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use shed_store::{atomic_write_json, Backup, WriteError};

use crate::address::normalize_client_ip;
use crate::identity::conversation_client_id;
use crate::py_strip;
use crate::record::ConversationRecord;

/// The folder every conversation is stored under.
///
/// The source's `MEMORY_DIR`: bare records at the top, and one folder per character
/// below them holding that character's transcripts of the same identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRoot {
    /// The storage root — `memory/` in the source's layout.
    pub memory_dir: PathBuf,
}

impl ConversationRoot {
    /// Where one conversation lives: the character's folder, or the root.
    ///
    /// `memory_dir/<slug>/<filename>` when a slug is given and
    /// `memory_dir/<filename>` when it is not — `None` and `""` alike, which is how
    /// this crate reads every optional string. The name is the browser id when the
    /// client sends one, or its address when it does not, so the file that holds a
    /// returning visitor's conversation is recognisable without a lookup table.
    ///
    /// The folder is not created here: `atomic_write_json` makes the parent directory
    /// on the way to writing, and reading a file in a folder that was never made is
    /// the absence the reader already answers with.
    #[must_use]
    pub fn record_path(&self, filename: &str, character_slug: Option<&str>) -> PathBuf {
        let Some(slug) = character_slug.filter(|slug| !slug.is_empty()) else {
            return self.memory_dir.join(filename);
        };
        self.memory_dir.join(slug).join(filename)
    }

    /// The record holding one identity's conversation, or `None`.
    ///
    /// The same identity can hold a conversation with more than one character, and
    /// each lives in that character's folder. The newest is the conversation as it
    /// stands: when a voice was changed mid-page, the later one is the one being had,
    /// and the earlier one is history.
    ///
    /// Scans `memory_dir/*/<filename>` and `memory_dir/<filename>`, and keeps the
    /// newest by modified time. The source's `max` over a glob answers a tie by
    /// whichever match it saw first; this sorts by `(mtime, path)` and takes the
    /// last, so the same folder on the same day gives the same answer twice
    /// (`docs/PARITY.md`). A file whose metadata cannot be read sorts before the ones
    /// that can be read rather than failing the whole lookup, which is the reading
    /// rule this module uses everywhere else.
    ///
    /// # Errors
    ///
    /// The [`io::ErrorKind`] of a directory that cannot be listed. A root that does
    /// not exist yet answers `None` — an app that has stored nothing has no record,
    /// which is not a failure — and so does a filename no folder carries.
    pub fn record_for_key(&self, filename: &str) -> io::Result<Option<PathBuf>> {
        if filename.is_empty() {
            return Ok(None);
        }

        let mut candidates: Vec<(Option<SystemTime>, PathBuf)> = Vec::new();
        push_candidate(&mut candidates, self.memory_dir.join(filename));

        match std::fs::read_dir(&self.memory_dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    push_candidate(&mut candidates, entry.path().join(filename));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        Ok(candidates.into_iter().max().map(|(_mtime, path)| path))
    }

    /// The record stored under `filename` at the root, or the empty record.
    ///
    /// A missing or empty name is the empty record rather than an error — the
    /// source's `if not conversation_key`. The character's folder is not a parameter
    /// here: a caller that speaks to a character composes the path through
    /// [`ConversationRoot::record_path`] or resolves one through
    /// [`ConversationRoot::record_for_key`] and reads it with
    /// [`read_conversation_record_file`].
    #[must_use]
    pub fn load_conversation_record(&self, filename: Option<&str>) -> ConversationRecord {
        match filename.filter(|name| !name.is_empty()) {
            Some(name) => read_conversation_record_file(&self.record_path(name, None)),
            None => ConversationRecord::default(),
        }
    }

    /// The style this conversation was last delivered in, or `""`.
    ///
    /// Read before a turn resolves its own style: with no explicit choice on the
    /// request, continuing the conversation means continuing the way it was being
    /// spoken. An empty answer leaves the decision to the person's wider habits, so
    /// absent and empty are the same thing to every caller and this returns
    /// [`String`] rather than making them distinguishable.
    #[must_use]
    pub fn conversation_delivery_style(&self, filename: Option<&str>) -> String {
        let record = self.load_conversation_record(filename);
        match record.delivery_style.as_deref() {
            Some(style) => py_strip(style).to_owned(),
            None => String::new(),
        }
    }

    /// Store `record` under `filename` at the root.
    ///
    /// An empty name stores nothing, as the source's `if not conversation_key`
    /// returns without writing: the caller that could not name the conversation has
    /// no file to write it to, and it already has the refusal from
    /// [`crate::conversation_filename`] to report.
    ///
    /// # Errors
    ///
    /// Whatever [`write_conversation_record_file`] reports for the path.
    pub fn save_conversation_record(
        &self,
        filename: &str,
        record: &ConversationRecord,
    ) -> Result<(), WriteError> {
        if filename.is_empty() {
            return Ok(());
        }
        write_conversation_record_file(&self.record_path(filename, None), record)
    }
}

/// One conversation record off disk, or the empty record.
///
/// A record that cannot be read is read as an absence rather than raising: no file,
/// a file that cannot be opened, a file that is not JSON, and a file whose JSON is
/// not a record all answer [`ConversationRecord::default()`]. See the module docs for
/// why this is not a `Result`.
#[must_use]
pub fn read_conversation_record_file(path: &Path) -> ConversationRecord {
    let Ok(text) = std::fs::read_to_string(path) else {
        return ConversationRecord::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Store a conversation record atomically, leaving no `.bak` beside it.
///
/// Two writers share this file — the turn and the profile worker — and a reader that
/// caught half a write would read a truncated conversation. `Backup::Skip` is the
/// source's `keep_backup=False`: a transcript is written every turn, and a second
/// copy of it beside each record preserves words already in the file they came from.
///
/// # Errors
///
/// [`WriteError`] when the directory cannot be made, the record cannot be rendered,
/// the temp file cannot be written, or the move into place is refused.
pub fn write_conversation_record_file(
    path: &Path,
    record: &ConversationRecord,
) -> Result<(), WriteError> {
    atomic_write_json(path, record, Backup::Skip)
}

/// Add `path` to `candidates` when something is there, with its modified time.
///
/// Metadata that cannot be read yields `None` for the time rather than an error:
/// such a path loses every comparison against one that can be dated, which is the
/// same answer as "not the newest conversation being had".
fn push_candidate(candidates: &mut Vec<(Option<SystemTime>, PathBuf)>, path: PathBuf) {
    let Ok(metadata) = path.metadata() else {
        return;
    };
    candidates.push((metadata.modified().ok(), path));
}

/// Every browser id already recorded against one address.
///
/// Ported from `agent.py:3820-3829`, with `iter_entity_client_records()` turned into
/// the pairs themselves: the client-file store is Stage 7's, so the caller supplies
/// `(client_id, client_ip)` and nothing here reads a clients directory (decision 7).
///
/// The comparison is between *stripped* spellings, as the source's
/// `str(record.get("client_ip") or "").strip() == wanted` compares them, so a record
/// written with padding still counts as seen. An empty address sees nobody — the
/// source's `if not wanted` arm, and the one that keeps an unaddressed request from
/// collecting every id in the store.
///
/// A `BTreeSet` rather than a `HashSet`: the answer is compared and printed by the
/// adoption guard that uses it, and a set with one order is one less thing for a test
/// to forget to sort.
#[must_use]
pub fn client_ids_seen_from_address(
    clients: &[(String, String)],
    address: &str,
) -> BTreeSet<String> {
    let wanted = py_strip(address);
    if wanted.is_empty() {
        return BTreeSet::new();
    }
    clients
        .iter()
        .filter(|(_, recorded)| py_strip(recorded) == wanted)
        .map(|(client_id, _)| client_id.clone())
        .collect()
}

/// Hand an address-keyed transcript to the one browser that owns it.
///
/// Ported from `claim_legacy_conversation` (`agent.py:3815-3841`), which is a
/// procedure returning `None`; this returns whether it adopted, and hands back the
/// rename's own failure rather than logging it (decision 7) — the caller is the one
/// that knows whether a turn may proceed on a fresh history.
///
/// Both names are filenames a caller resolved with [`ConversationRoot::record_path`],
/// so a character's folder is already part of the path and the guard needs no profile.
/// The browser id comes out of the *target* name and the address out of the *legacy*
/// name, which is the source's first guard re-expressed over the data this takes: a
/// target that is not a browser's, or a legacy name that is not an address, has
/// nothing to adopt, and that is settled before the disk is touched.
///
/// The guard order is the source's, and each position is load-bearing:
/// 1. the browser already has a history of its own → no. This is why "newest wins"
///    cannot replace the whole function: the two files must stay two files.
/// 2. there is no address-keyed record → no.
/// 3. any id recorded against that address other than this browser's → no. Once two
///    browsers share an address there is no deterministic answer to whose words the
///    transcript holds, so nobody takes it and each starts its own history.
///
/// Then `fs::rename`, which is `os.replace`: the legacy file is gone afterwards, so a
/// second browser arriving later is refused by arm 2 rather than arm 3 — and an
/// address-only client (one that has no target name to offer at all) never reaches
/// this function, because it has nothing to move a record *to*.
///
/// # Errors
///
/// The [`io::Error`] of the rename. Nothing else here does I/O that can fail: the
/// target's parent folder is the legacy record's own folder, which by arm 2 exists.
/// An `Err` means the legacy record is still where it was — `rename` either moved it
/// or did nothing.
pub fn claim_legacy_conversation(
    root: &ConversationRoot,
    filename: &str,
    legacy_filename: &str,
    clients: &[(String, String)],
) -> Result<bool, io::Error> {
    let Some(client_id) = conversation_client_id(Some(&file_stem(filename))) else {
        return Ok(false);
    };
    let Some(address) = legacy_address(legacy_filename) else {
        return Ok(false);
    };

    let target = root.record_path(filename, None);
    let legacy = root.record_path(legacy_filename, None);
    if target.exists() || !legacy.exists() {
        return Ok(false);
    }
    let seen = client_ids_seen_from_address(clients, &address);
    if !seen.iter().all(|id| *id == client_id) {
        return Ok(false);
    }

    std::fs::rename(&legacy, &target)?;
    Ok(true)
}

/// A stored file name without its extension, which is the conversation key it was
/// named from.
///
/// `Path::file_stem` would do this, but it also resolves `.` and `..` and drops a
/// trailing dot, and a name that odd is refused by [`crate::conversation_filename`]
/// rather than interpreted — so this is the same plain test the filename functions
/// make of the extension.
fn file_stem(filename: &str) -> String {
    match filename.strip_suffix(".json") {
        Some(stem) => stem.to_owned(),
        None => filename.to_owned(),
    }
}

/// The address a legacy file was named for, or `None` when it was not named for one.
///
/// The stored spelling writes an IPv6 address's colons as dashes, because Windows
/// refuses a colon in a file name, so the dashes go back first and the result is
/// normalized — the same reversal [`crate::names_a_conversation`] performs, for the
/// same reason. A name that does not come back as an address is a browser's own file,
/// or an old session record, or the character outline; none is anybody's legacy
/// transcript.
fn legacy_address(legacy_filename: &str) -> Option<String> {
    let stem = file_stem(legacy_filename);
    normalize_client_ip(Some(&stem.replace('-', ":")))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use crate::record::ConversationMessage;

    /// A directory of this run's own, made empty first so a crashed earlier run
    /// cannot leave a file that changes what an assertion sees.
    fn scratch(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let root = std::env::temp_dir().join(format!(
            "shed-conversation-store-{label}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("cannot make {}: {error}", root.display()));
        root
    }

    /// Take the run's scratch directory away again.
    fn tidy(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn message(role: &str, content: &str, timestamp: &str) -> ConversationMessage {
        ConversationMessage {
            role: role.to_owned(),
            content: content.to_owned(),
            timestamp: timestamp.to_owned(),
        }
    }

    fn record_spoken_in(style: Option<&str>) -> ConversationRecord {
        ConversationRecord {
            messages: vec![message(
                "assistant",
                "Come in — the kettle is already on.",
                "2026-10-27T12:00:00+00:00",
            )],
            summaries: Vec::new(),
            delivery_style: style.map(str::to_owned),
            user_details: None,
            user_details_covered_through: None,
        }
    }

    fn write(path: &Path, record: &ConversationRecord) {
        write_conversation_record_file(path, record)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    }

    /// Set a file's modified time to an absolute moment, so "newest" is a fact the
    /// test states rather than one the clock happens to arrange.
    fn stamp(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap_or_else(|error| panic!("cannot open {}: {error}", path.display()));
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap_or_else(|error| panic!("cannot stamp {}: {error}", path.display()));
    }

    #[test]
    fn the_empty_record_written_and_read_back_is_the_empty_record() {
        let root = scratch("round-trip");
        let path = root.join("client-ab12.json");

        write(&path, &ConversationRecord::default());

        assert_eq!(
            read_conversation_record_file(&path),
            ConversationRecord::default()
        );
        // The two required keys are written as the empty lists they are, so the file
        // is a record the source would also read.
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert!(
            text.contains("\"messages\": []") && text.contains("\"summaries\": []"),
            "stored record was: {text}"
        );
        // And no copy of it is kept beside the file it was copied from.
        assert!(!root.join("client-ab12.json.bak").exists());

        tidy(&root);
    }

    #[test]
    fn a_record_that_cannot_be_read_reads_as_the_empty_record() {
        let root = scratch("damaged");
        let half_written = root.join("client-cd34.json");
        std::fs::write(&half_written, b"{\"messages\": [")
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", half_written.display()));

        // A file that is half a write, and a file that is not there at all, are both
        // the absence rather than an error nobody watching could act on.
        assert_eq!(
            read_conversation_record_file(&half_written),
            ConversationRecord::default()
        );
        assert_eq!(
            read_conversation_record_file(&root.join("nowhere.json")),
            ConversationRecord::default()
        );
        // A directory is not a record either.
        assert_eq!(
            read_conversation_record_file(&root),
            ConversationRecord::default()
        );

        tidy(&root);
    }

    #[test]
    fn the_style_the_last_turn_was_spoken_in_survives_the_round_trip() {
        let root = scratch("style");
        let conversation = ConversationRoot {
            memory_dir: root.clone(),
        };
        let spoken = record_spoken_in(Some("  warm  "));
        conversation
            .save_conversation_record("client-ab12.json", &spoken)
            .unwrap_or_else(|error| panic!("save failed: {error}"));

        assert_eq!(
            conversation.conversation_delivery_style(Some("client-ab12.json")),
            "warm"
        );
        assert_eq!(
            conversation.load_conversation_record(Some("client-ab12.json")),
            spoken
        );

        // A record that carries no style, and no record at all, both answer "": the
        // file does not distinguish "spoken plainly" from "the field did not exist".
        conversation
            .save_conversation_record("client-cd34.json", &record_spoken_in(None))
            .unwrap_or_else(|error| panic!("save failed: {error}"));
        assert_eq!(
            conversation.conversation_delivery_style(Some("client-cd34.json")),
            ""
        );
        assert_eq!(conversation.conversation_delivery_style(Some("")), "");
        assert_eq!(conversation.conversation_delivery_style(None), "");
        assert_eq!(
            conversation.load_conversation_record(None),
            ConversationRecord::default()
        );

        // An empty name stores nothing, so nothing appears beside the two records.
        conversation
            .save_conversation_record("", &record_spoken_in(Some("dry")))
            .unwrap_or_else(|error| panic!("save failed: {error}"));
        assert_eq!(
            conversation.conversation_delivery_style(Some("")),
            "",
            "an empty name must not name a file"
        );

        tidy(&root);
    }

    #[test]
    fn a_shared_identity_resolves_to_the_newest_of_two_character_folders() {
        let root = scratch("newest");
        let filename = "client-ab12.json";
        let conversation = ConversationRoot {
            memory_dir: root.clone(),
        };
        let boris = root.join("boris").join(filename);
        let milla = root.join("milla").join(filename);
        write(&boris, &ConversationRecord::default());
        write(&milla, &ConversationRecord::default());
        stamp(&boris, 1_700_000_000);
        stamp(&milla, 1_700_000_600);

        let newest = conversation
            .record_for_key(filename)
            .unwrap_or_else(|error| panic!("lookup failed: {error}"));
        assert_eq!(newest, Some(milla), "the later voice is the conversation");

        // The bare record at the root is a match too, and wins when it is the newest.
        let bare = root.join(filename);
        write(&bare, &ConversationRecord::default());
        stamp(&bare, 1_700_001_200);
        let newest = conversation
            .record_for_key(filename)
            .unwrap_or_else(|error| panic!("lookup failed: {error}"));
        assert_eq!(newest, Some(bare));

        tidy(&root);

        // Nothing on disk — and no root at all — is the absence, not a failure.
        let newest = conversation
            .record_for_key(filename)
            .unwrap_or_else(|error| panic!("lookup failed: {error}"));
        assert_eq!(newest, None);
        let nothing = conversation
            .record_for_key("")
            .unwrap_or_else(|error| panic!("lookup failed: {error}"));
        assert_eq!(nothing, None);
    }
}
