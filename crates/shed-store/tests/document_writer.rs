//! `tests/test_entity_store_utils.py` — the document writer's half of that
//! suite, which is the half Stage 4 has a caller for.
//!
//! The suite's other class, `JsonlLogTests`, tests `append_jsonl` and
//! `read_jsonl_tail`. Neither has a production caller before Stage 7 — their
//! readers are the thought journal, facts and events (`agent.py:9717`–`10023`),
//! all `shed-mind`'s ground — so they are not ported here, and neither is the
//! code they test (`How_To_Code` principle 16). The suite splits across stages
//! the same way `test_text_utils` already does (plan §8.1, §9).
//!
//! Two of the cases below come from `tests/test_outbox.py:213-226` rather than
//! from this suite: `test_atomic_write_leaves_no_temp_files_behind` and
//! `test_atomic_write_replaces_rather_than_appends` are the outbox's tests of
//! the *same* writer, and the writer is what Stage 4 is porting.

//! The test binary links these crates through the library but names none of
//! them itself; the workspace's `unused_crate_dependencies` lint judges the
//! binary on its own uses.
use serde as _;
use tracing as _;
use uuid as _;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use shed_store::{atomic_write_json, backup_path, Backup, WriteError};

/// A directory of this run's own, removed first so a crashed earlier run cannot
/// leave a file that changes what the next assertion sees.
fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("shed-store-{}-{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap_or_else(|error| panic!("cannot make {root:?}: {error}"));
    root
}

/// Read a stored document back the way the app does.
fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

/// Every `*.tmp` still in `dir` — the litter a failed write must not leave.
fn temp_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot list {dir:?}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    names.sort();
    names
}

#[test]
fn a_second_write_keeps_the_previous_version() {
    // `test_by_default_a_second_write_keeps_the_previous_version`
    // (`test_entity_store_utils.py:32`): a mind-state record has to be
    // recoverable from something, so the copy is the default.
    let root = scratch("backup");
    let path = root.join("state.json");

    atomic_write_json(&path, &json!({"a": 1}), Backup::Keep)
        .unwrap_or_else(|error| panic!("first write failed: {error}"));
    atomic_write_json(&path, &json!({"a": 2}), Backup::Keep)
        .unwrap_or_else(|error| panic!("second write failed: {error}"));

    assert_eq!(read_json(&backup_path(&path)), json!({"a": 1}));
    assert_eq!(read_json(&path), json!({"a": 2}));
}

#[test]
fn a_caller_that_keeps_no_copy_leaves_no_backup() {
    // `test_a_caller_that_keeps_no_copy_leaves_no_backup` (line 41), which is
    // `write_conversation_record_file` (`agent.py:3919`): a transcript is
    // rewritten every turn, so a second copy of it is space spent preserving
    // words that are already in the file they were copied from.
    let root = scratch("no-backup");
    let path = root.join("state.json");

    atomic_write_json(&path, &json!({"a": 1}), Backup::Skip)
        .unwrap_or_else(|error| panic!("first write failed: {error}"));
    atomic_write_json(&path, &json!({"a": 2}), Backup::Skip)
        .unwrap_or_else(|error| panic!("second write failed: {error}"));

    assert!(!backup_path(&path).exists(), "a transcript keeps no copy");
    assert_eq!(read_json(&path), json!({"a": 2}));
}

#[test]
fn the_backup_is_named_after_the_whole_file() {
    // `Path(str(path) + ".bak")` (`entity_store.py:48`): the suffix goes on the
    // end of the name, so a conversation record keeps its `client-` prefix and
    // its `.json` in the copy's name too.
    let root = scratch("backup-name");
    assert_eq!(
        backup_path(&root.join("client-ab12cd34.json")).file_name(),
        Some(std::ffi::OsStr::new("client-ab12cd34.json.bak"))
    );
}

#[test]
fn a_copy_that_cannot_be_made_never_costs_the_write() {
    // The source's `except OSError: pass` around `shutil.copyfile`
    // (`entity_store.py:90-93`): losing the backup must not lose the document.
    // Here the copy is made to fail the way a real failure looks — the `.bak`
    // name already taken by something that is not a file.
    let root = scratch("backup-blocked");
    let path = root.join("state.json");
    std::fs::create_dir_all(backup_path(&path))
        .unwrap_or_else(|error| panic!("cannot block the backup: {error}"));

    atomic_write_json(&path, &json!({"a": 1}), Backup::Keep).unwrap_or_else(|error| {
        panic!("the blocked backup should not have failed the write: {error}")
    });

    assert_eq!(read_json(&path), json!({"a": 1}));
    assert_eq!(temp_files(&root), Vec::<String>::new());
}

#[test]
fn no_temp_files_are_left_behind() {
    // `test_atomic_write_leaves_no_temp_files_behind` (`test_outbox.py:213`).
    let root = scratch("litter");
    let path = root.join("state.json");

    atomic_write_json(&path, &json!({"a": 1}), Backup::Keep)
        .unwrap_or_else(|error| panic!("first write failed: {error}"));
    atomic_write_json(&path, &json!({"a": 2}), Backup::Keep)
        .unwrap_or_else(|error| panic!("second write failed: {error}"));

    assert_eq!(temp_files(&root), Vec::<String>::new());
    assert_eq!(read_json(&path), json!({"a": 2}));
}

#[test]
fn a_write_replaces_rather_than_appends() {
    // `test_atomic_write_replaces_rather_than_appends` (`test_outbox.py:222`):
    // the document gets shorter, and a writer that appended would leave the
    // tail of the old one behind.
    let root = scratch("replace");
    let path = root.join("state.json");

    atomic_write_json(
        &path,
        &json!({"items": (0..50).collect::<Vec<_>>()}),
        Backup::Keep,
    )
    .unwrap_or_else(|error| panic!("first write failed: {error}"));
    atomic_write_json(&path, &json!({"items": [1]}), Backup::Keep)
        .unwrap_or_else(|error| panic!("second write failed: {error}"));

    assert_eq!(read_json(&path), json!({"items": [1]}));
}

#[test]
fn the_parent_directory_is_created_rather_than_assumed() {
    // `path.parent.mkdir(parents=True, exist_ok=True)` (`entity_store.py:73`).
    // A character's conversation folder appears on that character's first turn,
    // and a writer that assumed it existed would fail the first message.
    let root = scratch("parents");
    let path = root.join("aria").join("client-ab12cd34.json");

    atomic_write_json(&path, &json!({"messages": []}), Backup::Skip)
        .unwrap_or_else(|error| panic!("write into a missing folder failed: {error}"));

    assert_eq!(read_json(&path), json!({"messages": []}));
}

#[test]
fn the_document_is_indented_by_two_and_keeps_its_characters() {
    // `json.dumps(payload, indent=2, ensure_ascii=False)` (`entity_store.py:74`).
    // The bytes are pinned rather than round-tripped, because a person reads
    // these files while reviewing a dry run, and `\u0161i\u0161ina` in a
    // conversation record is unreadable to them.
    //
    // Two facts here are the clone's, not the source's, and both are recorded in
    // `docs/PARITY.md`: the line endings are LF on every platform (the source
    // opens its file in text mode, so the same document is CRLF on Windows), and
    // the keys come out in order because a `serde_json::Value` object is sorted
    // where a Python dict keeps the order it was built in.
    let root = scratch("shape");
    let path = root.join("state.json");

    atomic_write_json(
        &path,
        &json!({"text": "Tišina u šupi.", "n": 2}),
        Backup::Skip,
    )
    .unwrap_or_else(|error| panic!("write failed: {error}"));

    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("cannot read: {error}"));
    assert_eq!(
        String::from_utf8(bytes).unwrap_or_else(|error| panic!("not utf-8: {error}")),
        "{\n  \"n\": 2,\n  \"text\": \"Tišina u šupi.\"\n}"
    );
}

#[test]
fn a_document_that_cannot_be_moved_leaves_no_temp_file() {
    // The source's `except BaseException` arm (`entity_store.py:96-101`) unlinks
    // the temp file and re-raises. Here the move is refused by a directory
    // already sitting on the target name, which fails the same way on both
    // platforms and needs no second process holding the file open.
    let root = scratch("blocked-replace");
    let path = root.join("state.json");
    std::fs::create_dir_all(&path)
        .unwrap_or_else(|error| panic!("cannot block the target: {error}"));

    let error = atomic_write_json(&path, &json!({"a": 1}), Backup::Skip)
        .err()
        .unwrap_or_else(|| panic!("a directory cannot be replaced by a file"));

    assert!(
        matches!(error, WriteError::Replace { .. }),
        "expected a refused move, got {error}"
    );
    // The message names both paths, because "cannot write" tells a person
    // nothing while "cannot move X onto Y" tells them the document was built and
    // the move was refused.
    assert!(error.to_string().contains("cannot move"), "{error}");
    assert!(error.to_string().contains("state.json"), "{error}");
    assert_eq!(temp_files(&root), Vec::<String>::new());
}

#[test]
fn a_serialisable_failure_is_reported_before_any_file_exists() {
    // The bytes are rendered before the temp file is created, as they are in the
    // source (`entity_store.py:74` precedes the `mkstemp` at 76), so a payload
    // that cannot be rendered leaves nothing behind. The payload that cannot be
    // rendered in Rust is a map key that is not a string — a case the source
    // cannot even build, since `json.dumps` rejects it at the same point.
    let root = scratch("serialize");
    let path = root.join("state.json");
    let keyed: std::collections::HashMap<Vec<u8>, i32> = [(vec![0xff_u8], 1)].into_iter().collect();

    let error = atomic_write_json(&path, &keyed, Backup::Skip)
        .err()
        .unwrap_or_else(|| panic!("a non-utf8 key cannot become a JSON object key"));

    assert!(
        matches!(error, WriteError::Serialize { .. }),
        "expected a serialising failure, got {error}"
    );
    assert!(!path.exists(), "the document was never written");
    assert_eq!(temp_files(&root), Vec::<String>::new());
}
