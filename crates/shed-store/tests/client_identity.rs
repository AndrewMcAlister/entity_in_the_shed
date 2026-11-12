//! The client id: what a browser may name itself with, and what the server
//! invents when it does not say.
//!
//! Ports the identity-safety cases of `tests/test_outbox.py:184-209`
//! (`test_a_traversal_client_id_cannot_escape_the_directory`,
//! `test_a_well_formed_id_is_kept_and_a_short_one_is_replaced`). That suite
//! belongs to `shed-entity` at Stage 7 in the plan's test map (§9), but the
//! cases below test `normalize_client_id`/`resolve_client_id`, which Stage 4 is
//! porting now because the browser chat flow reads them before it resolves a
//! conversation (`agent.py:9166-9167`) — and untested validation that keeps
//! attacker-controlled text out of a filename is principle 15's whole reason for
//! existing.
//!
//! The suite's third case there, `test_even_an_accepted_id_is_hashed_into_the_filename`,
//! is not here: it tests `client_file_path`, which Stage 7 writes.

//! The test binary links these crates through the library but names none of
//! them itself; the workspace's `unused_crate_dependencies` lint judges the
//! binary on its own uses.
use serde as _;
use serde_json as _;
use tracing as _;
use uuid as _;

use shed_store::{mint_client_id, normalize_client_id, resolve_client_id};

#[test]
fn a_traversal_id_is_rejected_rather_than_sanitised() {
    // `test_a_traversal_client_id_cannot_escape_the_directory`
    // (`test_outbox.py:186`): the hostile value is refused outright, so the
    // caller mints a fresh identity instead of storing a record outside the
    // directory that holds records.
    let hostile = "../../etc/passwd";
    assert_eq!(normalize_client_id(hostile), "");
    assert_ne!(resolve_client_id(hostile), hostile);
}

#[test]
fn a_well_formed_id_is_kept_and_a_short_one_is_replaced() {
    // `test_a_well_formed_id_is_kept_and_a_short_one_is_replaced`
    // (`test_outbox.py:201`).
    assert_eq!(resolve_client_id("client-abcdefgh"), "client-abcdefgh");
    // Too short to be one of ours, so it is replaced rather than trusted.
    assert_ne!(resolve_client_id("abc"), "abc");
    // Nothing at all — a cleared `localStorage` — still gets a working identity
    // rather than an error, because the alternative breaks chat outright to
    // protect a feature the client was not using.
    assert!(!resolve_client_id("").is_empty());
    // The source's fourth case is `resolve_client_id(None)`. A Rust caller has
    // no `None` to pass: it reads the body field through `shed_text::text_or_empty`,
    // which is where an absent key and an empty one become the same empty string.
    assert_eq!(normalize_client_id(""), "");
}

#[test]
fn the_id_is_stripped_before_it_is_tested() {
    // `str(raw or "").strip()` (`entity_store.py:232`): a value sent with
    // padding is the same identity as the value without it, so a chat turn
    // continues the conversation it belongs to rather than starting a new one.
    assert_eq!(
        normalize_client_id(" \t client-abcdefgh \r\n"),
        "client-abcdefgh"
    );
    // Python's `str.strip()` also trims `\x1c`–`\x1f`, which Rust's
    // `char::is_whitespace` does not consider whitespace.
    assert_eq!(
        normalize_client_id("\u{1c}client-abcdefgh\u{1f}"),
        "client-abcdefgh"
    );
    // Only the ends are stripped: an id with a space inside it is not one.
    assert_eq!(normalize_client_id("client abcdefgh"), "");
}

#[test]
fn the_alphabet_and_the_length_bound_are_the_whole_rule() {
    // `^[A-Za-z0-9_-]{8,64}$` (`entity_store.py:30`).
    for accepted in [
        "abcdefgh", // the shortest legal id
        "client-abcdefgh",
        "client_abcdefgh",
        "AbCdEfGh0123456789",
        &"a".repeat(64), // the longest legal id
    ] {
        assert_eq!(normalize_client_id(accepted), accepted, "{accepted}");
    }
    for rejected in [
        "",
        "abcdefg",          // one short of the bound
        &"a".repeat(65),    // one over it
        "client-abcdefgh.", // the traversal alphabet
        "client-abcdefgh/..",
        "client abcdefgh",  // a space inside
        "client-abcdefghé", // non-ASCII, in range either way
    ] {
        assert_eq!(
            normalize_client_id(rejected),
            "",
            "{rejected} should be refused"
        );
    }
}

#[test]
fn a_minted_id_is_itself_a_valid_one() {
    // `uuid4().hex` is thirty-two lower-case hex characters, which is inside the
    // pattern. That is what makes `resolve_client_id` safe to feed straight into
    // a path: whatever it returns, it returns something the pattern accepts.
    let minted = mint_client_id();
    assert_eq!(minted.len(), 32, "{minted}");
    assert_eq!(normalize_client_id(&minted), minted);
    assert_ne!(mint_client_id(), minted, "two mints are two identities");
}

#[test]
fn resolving_the_same_id_twice_gives_the_same_id() {
    // The property the conversation key depends on: the same request always
    // names the same file, with no part of it depending on the clock or chance
    // (`utils/conversation_history.py:148-161`).
    assert_eq!(
        resolve_client_id("client-abcdefgh"),
        resolve_client_id("client-abcdefgh")
    );
}
