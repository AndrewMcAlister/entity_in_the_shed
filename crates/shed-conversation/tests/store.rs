//! Who reads which transcript, and what happens to the old address files.
//!
//! `ConversationOwnershipTests` (`tests/test_conversation_identity.py:27-164`), ported
//! whole. The Python class patches `agent.MEMORY_DIR` and `agent.OUTBOX_CLIENTS_DIR`
//! to a temporary folder; the clone needs no patching because
//! [`ConversationRoot`] takes its root as a value (decision 7 also turned the clients
//! directory into the `(client_id, client_ip)` pairs the caller supplies, so
//! `seed_client` writes nothing to disk at all — the pair *is* the seed).
//!
//! The fixtures come over unchanged, including the choice of who is "known" at the
//! address in each case: that choice is the whole content of four of the eight tests.
//!
//! One substitution, and it is 4e's absence rather than a change: the Python
//! `append_conversation_message` used to fill a record does not exist yet, so the
//! tests write their one-message record with [`write_conversation_record_file`]. What
//! each test asserts is unchanged — the words that come back out of the file.

// The test binary links these crates through the library but names none of them
// itself; the workspace's `unused_crate_dependencies` lint judges the binary on its
// own uses.
use chrono as _;
use serde as _;
use serde_json as _;
use shed_prompts as _;
use shed_store as _;

use std::path::Path;

use shed_conversation::{
    claim_legacy_conversation, client_ids_seen_from_address, conversation_filename,
    conversation_identity, read_conversation_record_file, write_conversation_record_file,
    ConversationMessage, ConversationRecord, ConversationRoot,
};

/// The address both browsers of the adoption tests were last seen from.
const CLIENT_IP: &str = "203.0.113.7";
/// Another address, for the browser that moved and kept its key.
const OTHER_IP: &str = "198.51.100.9";
/// A third address, for a browser that has nothing to do with either of the above.
const THIRD_IP: &str = "203.0.113.99";
/// Two browser ids of the shape the page mints: sixteen hex characters.
const FIRST_CLIENT: &str = "aaaaaaaaaaaaaaaa";
const SECOND_CLIENT: &str = "bbbbbbbbbbbbbbbb";
/// The moment every seeded row carries, so a stored record is byte-comparable.
const STAMP: &str = "2026-10-27T09:00:00+00:00";

/// A memory root of this test's own, emptied first so a crashed earlier run cannot
/// leave a file that changes what an assertion sees.
fn scratch(label: &str) -> ConversationRoot {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let root = std::env::temp_dir().join(format!(
        "shed-conversation-ownership-{label}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)
        .unwrap_or_else(|error| panic!("cannot make {}: {error}", root.display()));
    ConversationRoot { memory_dir: root }
}

/// Take the run's scratch directory away again.
fn tidy(root: &ConversationRoot) {
    let _ = std::fs::remove_dir_all(&root.memory_dir);
}

/// The filename a key is stored under — the door every path in these tests goes
/// through, because a key that names nothing must fail here rather than three layers
/// down (`store.rs`'s module doc says why).
fn filename(key: &str) -> String {
    match conversation_filename(key) {
        Ok(name) => name,
        Err(error) => panic!("{key} should name a conversation: {error:?}"),
    }
}

/// The key a request names: `identity.rs`'s answer, which these tests exercise as the
/// source does rather than re-pinning (its own tests are in `identity.rs`).
fn identity(client_id: &str, client_ip: &str) -> String {
    match conversation_identity(Some(client_id), Some(client_ip)) {
        Some(key) => key,
        None => panic!("{client_id:?} at {client_ip:?} should name a conversation"),
    }
}

/// A browser the mind has already met, with the address it was last seen from — the
/// pair `seed_client` writes as a client file, which is all this slice reads of it.
fn seed(client_id: &str, client_ip: &str) -> (String, String) {
    (client_id.to_owned(), client_ip.to_owned())
}

/// A record of one row, written the way the app writes it.
fn write_record(path: &Path, content: &str) {
    let record = ConversationRecord {
        messages: vec![ConversationMessage {
            role: "user".to_owned(),
            content: content.to_owned(),
            timestamp: STAMP.to_owned(),
        }],
        summaries: Vec::new(),
        delivery_style: None,
        user_details: None,
        user_details_covered_through: None,
    };
    write_conversation_record_file(path, &record)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

/// The contents of a stored conversation, in stored order — how the source asserts on
/// a transcript, `["the first browser's words"]` rather than a record to dig apart.
fn contents(root: &ConversationRoot, name: &str) -> Vec<String> {
    root.load_conversation_record(Some(name))
        .messages
        .iter()
        .map(|row| row.content.clone())
        .collect()
}

/// The claim, run and unwrapped: these tests are about *whether* it adopted, and a
/// rename that fails in a temporary directory is a broken test rather than a result.
fn claim(
    root: &ConversationRoot,
    key: &str,
    legacy_name: &str,
    clients: &[(String, String)],
) -> bool {
    match claim_legacy_conversation(root, &filename(key), legacy_name, clients) {
        Ok(adopted) => adopted,
        Err(error) => panic!("the adoption should reach its guards: {error}"),
    }
}

// -- keying -----------------------------------------------------------------

#[test]
fn two_browsers_at_one_address_keep_separate_conversations() {
    let root = scratch("two-browsers");
    let first = identity(FIRST_CLIENT, CLIENT_IP);
    let second = identity(SECOND_CLIENT, CLIENT_IP);

    write_record(
        &root.record_path(&filename(&first), None),
        "the first browser's words",
    );

    assert_eq!(
        contents(&root, &filename(&first)),
        ["the first browser's words"]
    );
    assert!(
        contents(&root, &filename(&second)).is_empty(),
        "the second browser at the same address has no part of the first one's words"
    );
    assert_ne!(
        root.record_path(&filename(&first), None),
        root.record_path(&filename(&second), None)
    );

    tidy(&root);
}

#[test]
fn a_browser_keeps_one_key_when_its_address_changes() {
    let root = scratch("moved");
    let before = identity(FIRST_CLIENT, CLIENT_IP);
    let after = identity(FIRST_CLIENT, OTHER_IP);

    assert_eq!(before, after);
    assert_eq!(
        root.record_path(&filename(&before), None),
        root.record_path(&filename(&after), None)
    );

    tidy(&root);
}

#[test]
fn a_turn_without_a_browser_id_falls_back_to_its_address() {
    let root = scratch("address-only");
    let key = identity("", CLIENT_IP);

    assert_eq!(key, CLIENT_IP);
    assert_eq!(filename(&key), format!("{CLIENT_IP}.json"));
    // The path is the root's own file, not a folder's: an address-keyed record is
    // what every transcript was before browser ids, which is what the next tests move.
    assert_eq!(
        root.record_path(&filename(&key), None),
        root.memory_dir.join(format!("{CLIENT_IP}.json"))
    );

    tidy(&root);
}

// -- the move off address keys ---------------------------------------------

#[test]
fn a_legacy_address_record_is_adopted_by_its_only_known_browser() {
    let root = scratch("adopted");
    let legacy = root.memory_dir.join(format!("{CLIENT_IP}.json"));
    write_record(&legacy, "the old record's words");
    let clients = [seed(FIRST_CLIENT, CLIENT_IP)];
    let key = identity(FIRST_CLIENT, CLIENT_IP);

    assert!(claim(&root, &key, &format!("{CLIENT_IP}.json"), &clients));

    assert!(!legacy.exists(), "the record moved, it was not copied");
    assert_eq!(
        contents(&root, &filename(&key)),
        ["the old record's words"],
        "the browser inherits the transcript rather than watching it go"
    );

    tidy(&root);
}

#[test]
fn an_unclaimed_address_record_may_be_taken_by_the_first_browser() {
    // No client record names the address yet, so nothing says someone else owns the
    // words. The browser in front of the person is the only candidate, and leaving the
    // transcript to expire would lose it.
    let root = scratch("unclaimed");
    let legacy = root.memory_dir.join(format!("{CLIENT_IP}.json"));
    write_record(&legacy, "the old record's words");
    let key = identity(FIRST_CLIENT, CLIENT_IP);

    assert!(claim(&root, &key, &format!("{CLIENT_IP}.json"), &[]));

    assert!(!legacy.exists());
    assert!(root.record_path(&filename(&key), None).exists());

    tidy(&root);
}

#[test]
fn a_shared_address_leaves_the_legacy_record_to_no_one() {
    // Two browser ids have been seen at the address, so there is no deterministic
    // answer to whose transcript it is. Neither inherits it.
    let root = scratch("shared");
    let legacy = root.memory_dir.join(format!("{CLIENT_IP}.json"));
    write_record(&legacy, "someone's words");
    let clients = [
        seed(FIRST_CLIENT, CLIENT_IP),
        seed(SECOND_CLIENT, CLIENT_IP),
    ];
    let key = identity(FIRST_CLIENT, CLIENT_IP);

    assert!(!claim(&root, &key, &format!("{CLIENT_IP}.json"), &clients));

    assert!(legacy.exists());
    assert!(!root.record_path(&filename(&key), None).exists());

    tidy(&root);
}

#[test]
fn a_browser_that_already_has_history_never_picks_up_the_legacy_record() {
    let root = scratch("has-history");
    let legacy = root.memory_dir.join(format!("{CLIENT_IP}.json"));
    write_record(&legacy, "the old record's words");
    let clients = [seed(FIRST_CLIENT, CLIENT_IP)];
    let key = identity(FIRST_CLIENT, CLIENT_IP);
    let own = root.record_path(&filename(&key), None);
    write_record(&own, "this browser's own words");

    assert!(!claim(&root, &key, &format!("{CLIENT_IP}.json"), &clients));

    // The guard that decides this is the first one, and it is why the whole function
    // cannot be a "newest file wins" read: two transcripts must stay two transcripts.
    assert!(legacy.exists());
    assert_eq!(
        contents(&root, &filename(&key)),
        ["this browser's own words"]
    );

    tidy(&root);
}

#[test]
fn an_address_only_client_never_claims_a_legacy_record() {
    // A turn with no browser id has no `client-` target to move a record to, and the
    // source's first guard refuses it before the disk is touched. Here that is the
    // target name refusing to yield a browser id.
    let root = scratch("no-id");
    let legacy = root.memory_dir.join(format!("{CLIENT_IP}.json"));
    write_record(&legacy, "the old record's words");
    let clients = [seed(FIRST_CLIENT, CLIENT_IP)];

    assert!(!claim(
        &root,
        CLIENT_IP,
        &format!("{CLIENT_IP}.json"),
        &clients
    ));

    assert!(legacy.exists());
    // And the refusal is not a half-move: the bytes are still where they were.
    assert_eq!(
        contents(&root, &format!("{CLIENT_IP}.json")),
        ["the old record's words"]
    );

    tidy(&root);
}

// -- the two halves of the file the record is -------------------------------

#[test]
fn a_record_written_before_the_optional_fields_existed_loads_and_is_rewritten_without_them() {
    // The slice's `Done when`, pinned at the file level: `tests/records.rs` pins the
    // same rule against the type, and this pins it against a file on disk, which is
    // where the app actually meets it.
    let root = scratch("old-record");
    let name = format!("client-{FIRST_CLIENT}.json");
    let path = root.memory_dir.join(&name);
    let written_before = r#"{"messages":[{"role":"user","content":"an old line","timestamp":"2026-10-27T09:00:00+00:00"}],"summaries":[]}"#;
    std::fs::write(&path, written_before)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));

    let record = root.load_conversation_record(Some(&name));
    assert_eq!(record.messages.len(), 1);
    assert_eq!(record.delivery_style, None);
    assert_eq!(record.user_details, None);
    assert_eq!(record.user_details_covered_through, None);

    write_conversation_record_file(&path, &record)
        .unwrap_or_else(|error| panic!("cannot rewrite {}: {error}", path.display()));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert!(
        !text.contains("delivery_style")
            && !text.contains("user_details")
            && !text.contains("null"),
        "an absent field is absent, not null: {text}"
    );

    tidy(&root);
}

#[test]
fn a_damaged_record_is_left_alone() {
    // `test_a_damaged_record_is_left_alone`'s reading half (`4e` trims, and the
    // trimming half of that test is 4e's): a file nobody can read reads as the empty
    // record, and the read does not "fix" it by writing over it. A conversation the
    // app cannot parse may still be readable by a person, and repairing it by
    // overwriting it with nothing is the one thing a reader must never do.
    let root = scratch("damaged");
    let name = format!("client-{SECOND_CLIENT}.json");
    let path = root.memory_dir.join(&name);
    let damaged = b"{\"messages\": [{\"role\": \"user\", ";
    std::fs::write(&path, damaged)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));

    assert_eq!(
        read_conversation_record_file(&path),
        ConversationRecord::default()
    );
    assert_eq!(root.conversation_delivery_style(Some(&name)), "");

    let after = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert_eq!(
        after, damaged,
        "the read left the file exactly as it found it"
    );
    // No `.bak`, and no temp file left beside it either: nothing wrote at all.
    assert!(!root.memory_dir.join(format!("{name}.bak")).exists());
    let leftovers: Vec<String> = std::fs::read_dir(&root.memory_dir)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", root.memory_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != &format!("client-{SECOND_CLIENT}.json"))
        .collect();
    assert!(leftovers.is_empty(), "a read writes nothing: {leftovers:?}");

    tidy(&root);
}

#[test]
fn the_style_the_last_turn_was_spoken_in_survives_the_round_trip() {
    // `test_trimming_a_conversation_keeps_the_style_it_was_spoken_in`, the style-read
    // half only: the stored style comes back, and a record with none answers `""` so
    // the caller falls back to the person's wider habits.
    let root = scratch("style");
    let warm = format!("client-{FIRST_CLIENT}.json");
    let plain = format!("client-{SECOND_CLIENT}.json");
    let record = ConversationRecord {
        messages: Vec::new(),
        summaries: Vec::new(),
        delivery_style: Some("  warm  ".to_owned()),
        user_details: None,
        user_details_covered_through: None,
    };
    write_conversation_record_file(&root.memory_dir.join(&warm), &record)
        .unwrap_or_else(|error| panic!("cannot write: {error}"));

    assert_eq!(root.conversation_delivery_style(Some(&warm)), "warm");
    assert_eq!(root.conversation_delivery_style(Some(&plain)), "");
    assert_eq!(root.conversation_delivery_style(None), "");

    tidy(&root);
}

#[test]
fn the_ids_seen_at_an_address_are_the_ones_recorded_there() {
    // `client_ids_seen_from_address` is the guard the four adoption tests lean on, and
    // decision 7 made it public API, so it is pinned directly rather than only
    // through what refuses to adopt. Two of its three branches are the source's
    // `str(... or "").strip()` idioms, and both are the kind of thing a port drops:
    // an address nobody recorded is *empty*, not "everybody", and a record written
    // with padding is still the same address.
    let clients = [
        seed(FIRST_CLIENT, CLIENT_IP),
        seed(SECOND_CLIENT, "  198.51.100.9  "),
        seed("c".repeat(16).as_str(), THIRD_IP),
    ];

    let seen = client_ids_seen_from_address(&clients, CLIENT_IP);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen.contains(FIRST_CLIENT), "{seen:?}");

    // Padding on the *stored* side, and the padded spelling asked for: the strip is
    // applied to both, so the record counts as seen at its own address.
    let padded = client_ids_seen_from_address(&clients, "198.51.100.9");
    assert_eq!(padded.len(), 1, "{padded:?}");
    assert!(padded.contains(SECOND_CLIENT), "{padded:?}");

    // The source's `if not wanted` arm: an unaddressed request collects nobody, rather
    // than matching every record whose own address is also blank.
    let blank = [seed("d".repeat(16).as_str(), "")];
    assert!(
        client_ids_seen_from_address(&blank, "").is_empty(),
        "an empty address sees nobody"
    );
    assert!(client_ids_seen_from_address(&clients, "").is_empty());

    // An address nothing was recorded from is the empty set, which is what lets an
    // unclaimed record be taken at all.
    assert!(client_ids_seen_from_address(&clients, "192.0.2.5").is_empty());

    // The one line of the port no test reaches, stated rather than quietly skipped:
    // the rename's `Err` arm. Nothing short of an ACL denies a move between two paths
    // in a directory the process can write, and a test that spent its existence
    // arranging its own premise through OS trivia would be worth less than the four
    // guard tests above. What the arm does is written in the function's `# Errors`:
    // the failure is handed back, and the record is left where it was.
}
