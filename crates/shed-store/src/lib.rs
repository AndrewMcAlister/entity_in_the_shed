//! Durable JSON storage: a write a reader either sees whole or not at all, and
//! the client id that names a browser's files.
//!
//! Ported from `utils/entity_store.py`, which is pure stdlib — no Flask, no
//! `agent` import — and this crate keeps that purity: it knows no routes, no
//! settings struct and no model, only paths and bytes.
//!
//! ## Why a half-written file is the failure this primitive exists for
//!
//! Two writers share a conversation record — the turn and the profile worker —
//! and a reader that catches the file between "truncated" and "complete" reads a
//! conversation that lost its middle. So every write here goes to a temp file in
//! the *same directory* (a temp in the system temp dir would be a cross-volume
//! copy, which `rename` cannot make atomic) and is moved into place in one
//! operation. A reader sees the old file or the new one, never either half.
//!
//! ## What is here, and what is waiting for its first caller
//!
//! | Source | Where it goes |
//! |---|---|
//! | `atomic_write_json` (`entity_store.py:62`), `backup_path` (46) | here, in `json_doc.rs` — Stage 4, whose caller is `write_conversation_record_file` (`agent.py:3910`) |
//! | `normalize_client_id` (230), `mint_client_id` (236), `resolve_client_id` (240) | here, in `client_id.rs` — Stage 4, whose caller is the browser chat flow (`agent.py:9166-9167`) and `conversation_identity` (`utils/conversation_history.py:158`) |
//! | `atomic_write_lines` (104), `read_jsonl_tail` (142), `append_jsonl` (170) | Stage 7, in a `jsonl_log` module: their only callers are the thought journal, facts and events (9717-10049), which is `shed-mind`'s ground (§8.1: `shed-store` is filled at 4 for conversation files and at 7 for mind/client/outbox files) |
//! | `load_json_with_recovery` (193), `CorruptStateError` (37), `quarantine_path` (51), `_quarantine` (186) | Stage 7, in a `recovery` module: their callers are the client, user, goals, state and outbox records (9654-10805), all Stage 7 |
//! | `client_file_path` (251) | Stage 7, with the outbox and mind-user directories (9644, 9851). It hashes the id with `hashlib.sha1`, which this workspace does not carry (only `sha2`) — the digest is a filename inside the clone's own storage root, never a wire value, so that stage decides between a `sha1` dependency and a `sha2` digest of its own rather than this one guessing early |
//! | `EntityStoreError` (33) | nowhere as its own type: it exists only as the base of `CorruptStateError`, and Rust has no inheritance to mirror |
//! | `fsync` on both writers (84, 130) | not a parameter: every caller in the source leaves it `True`, so the clone syncs and says nothing. `keep_backup` stays a parameter because the source really does pass both values — `False` at 3919, the default at 9660-10234 |
//!
//! ## The `str(value or "")` half stays at the call site
//!
//! `normalize_client_id` takes a `&str`, not a JSON value, because the layer map
//! (`shed-gates`'s `LAYERS`) lets `shed-store` reach `shed-config` and
//! `shed-json` but not `shed-text`, and the falsy-then-`str()` idiom lives in
//! `shed-text`. So a route reads the body field through `shed_text::text_or_empty`
//! and hands the text here; the validation — and the reason it exists, an id that
//! becomes a filename is untrusted input — is all in this crate.

mod client_id;
mod json_doc;

pub use client_id::{mint_client_id, normalize_client_id, resolve_client_id};
pub use json_doc::{atomic_write_json, backup_path, Backup, WriteError};
