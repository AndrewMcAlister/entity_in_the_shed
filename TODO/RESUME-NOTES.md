# Resume Notes — 2026-11-21 (slice 4f-1 committed)

Rename this file with a `_DONE` suffix once Stage 9 closes (same time as PLAN-Rust-Clone.md), per principle 21.
Update this file at every milestone: each stage-slice commit, and any decision
or discovery a resuming session would otherwise have to re-derive.

## Where we are

- **SLICE 4f-1 IS COMMITTED** (`3298fee`, on `main`, 2026-11-21, on top of `df68519`, the crate
  scaffold): the whole reply layer — `crates/shed-chat/src/{lib,reply_text,quality,document,
  stream_scanner,splitter,streamed,publisher}.rs` plus seven integration test files, every one of
  them driven by `tests/fixtures/*.json` captured from the running app. Gate green on the committed
  tree: fmt, clippy `--workspace --all-targets --all-features -- -D warnings`, **670 Rust passed /
  0 failed**, **250 JS passed**. The 670 includes 177 in shed-chat: quality 93, splitter 25,
  stream_scanner 24, publisher 20, document 7, reply_text 2, lib units 6. The reference test files
  hold 136 tests (78 `test_reply_utils`, 21 `test_sentence_splitter`, 20
  `test_reply_stream_scanner`, 17 `test_reply_stream_publisher`) and all 136 are ported; the Rust
  count is higher because the constant tables and the fixture-driven sweeps are each their own
  test. `PublishFn` is re-exported from `lib.rs` for the publisher tests.
  `docs/PARITY.md` carries the reply-layer divergence rows; `docs/GUARD_CATALOGUE.md` carries the
  first two runtime-guard entries — reply completeness/usability and the prose tool-call envelope —
  with their table rows flipped to **ported**.
  **Next is 4f-2**: the reply *loop* around these pieces (the `run_agent` reply region, the turn
  gate, the duplicate-event guard). `TODO/SLICE-4f-BRIEF.md` is the scoping brief for the split.
  `origin/main` is at `df68519`, so `3298fee` and the notes commit that follows it are unpushed.

  **Two real port bugs, both found by the fixtures and both fixed in `src/`** — the reason to let
  the fixture win over a reading of the Python:
  - `reply_text::clause_spacing` was transcribed with the wrong class
    (`([,;:])(?=[A-Za-z])`); the source is `([,;:])(?=(?:["'(\[{])?[A-Za-z])`, so a comma before a
    quote or bracket was not being spaced.
  - `stream_scanner`'s `State::Fence` arm dropped the reference's `character.isspace()` test and
    aborted on the space after ` ``` `, which killed **every** ` ```json\n{…}` reply. The scanner
    fixtures are the only thing that caught it: the unit reading of the code looked fine.

  **Reading the reference app (`D:\Entity-In-The-Shed`, hyphen) is a trap.** `read` / `glob` /
  `grep` against that path returned *fabricated* file contents throughout 4f-1 — plausible Python
  that was not the file. The only trustworthy reads there are `python` run against a `.py` file and
  `shutil.copyfile` into `C:\Users\AndrewBot\AppData\Local\Temp\opencode\`, which is where the
  four reference test files and the verified line map live. The clone (`D:\Entity_In_The_Shed`,
  underscore) is fine. The same session fabricated a crate name (`shed-support`; there is no such
  crate — `py_repr` / `py_str` / `text_or_empty` are in `shed-text`) and fabricated `agent.py` line
  citations, so every provenance comment in `src/` was re-derived against the real file. Trust the
  brief's *paste* of the source; verify anything recalled from memory, including line numbers.

  **The verified line map** (re-derived, use it instead of re-searching): `agent.py` —
  `COMMON_NON_TERMINAL_ABBREVIATIONS` 283-296, `normalize_reply_text` 5504-5547,
  `markdown_to_ssml` 5550-5577, `JsonReplyStreamScanner` 5592-5799, `_DOTTED_ACRONYM_RE` 5809,
  `SentenceSplitter` 5819-5939 (its only `re` call is the trailing-token search at 5905),
  `_streamed_prefix_length` 5942-5960, `split_reply_after_stream` 5963-5976,
  `strip_streamed_prefix` 5979-5985, `ReplyStreamPublisher` 5988-6031, `REPLY_DOCUMENT_KEYS` 6037,
  `_parse_structured_candidate` 6040-6068, `_parse_model_reply_document` 6071-6136 (regex fallbacks
  6118-6134), `reply_without_prose_tool_calls` 6139-6146, `settle_prose_tool_calls` 6149-6191,
  `parse_model_json_content` 6194-6202. `utils/reply.py` — constants 16/25/28-46/241-244/250,
  functions 49-389 (`_is_duplicate_event` 384-389); `agent.py:157` imports all 14 public names.

  **What the fixtures already answer** (read them, but these were checked and re-deriving them
  costs a session):
  - `JsonReplyStreamScanner::MAX_PREAMBLE_CHARS` = **256**, `KEYS` = `("reply", "response")`.
  - `SentenceSplitter::MIN_SENTENCE_CHARS` = **12**, `MAX_SENTENCE_CHARS` = **240**, and
    `COMMON_NON_TERMINAL_ABBREVIATIONS` is **12 entries, stored lower-cased with no trailing dot**:
    `dr, e.g, etc, i.e, jr, mr, mrs, ms, …` (so the lookup compares a lower-cased word with the
    dot stripped — ported exactly, it is why `Mr.` is not a terminator).
  - **The scanner is chunk-independent**: `same_as_whole` is true for all 26 documents × 4 chunk
    sequences, including one-character-at-a-time and the two seeded splits, so the Rust
    chunk-independence assertion is meaningful, not a tautology.
  - **An abort is terminal**: after it, `feed()` returns `""` forever and `reply` stays empty.
  - **The publisher never speaks prose that precedes a JSON document.** `["Let me look. ",
    '{"tool_calls": [], "reply": "Later."}']` publishes **nothing** and aborts — the leading prose
    is still inside the 256-char preamble window when the `{` arrives, so it is swallowed. The Rust
    port must not "helpfully" flush the preamble on abort.
  - **`finish()` on a half-written document flushes a mid-word fragment**: cutting
    `{"reply": "First sentence arrives now. Second one is later on.", …}` in half publishes
    `["First sentence arrives now.", "Se"]`. Reference behaviour, pinned in `publisher.json` →
    `hand_built` → `half_a_document`; do not clean it up.
  - `_streamed_prefix_length` returns `None` when the spoken text is not a prefix of the reply
    (that `None` drives `split_reply_after_stream`'s `replaced` flag), and it compares
    **whitespace-normalised**: `"One two three.   Four five six.\nSeven eight nine."` against
    `"One two three. Four five six."` gives 31, not 27.
  - `settle_prose_tool_calls` logged **no** warnings for any of the 33 captured documents — the
    warning path needs a document that carries tool calls *outside* the JSON envelope (the
    `{tool_calls}…{/tool_calls}` prose form). `document.json` has those shapes; the Rust guard
    warns there too, and the fixture's `warnings` rows are deliberately **not** asserted (they are
    logger payloads; `shed-chat` emits the same text through `tracing::warn!`).

  **Decisions already taken, not to be re-litigated:**
  - **`fancy-regex`, not `regex`.** `normalize_reply_text`'s comma rule is a lookahead,
    `markdown_to_ssml` uses named groups inside a substitution callback, and the splitter uses lazy
    `*?`. `fancy-regex` is already a workspace dependency, so this adds no resolution risk.
  - **Chunk sequences are seeded, not ad hoc** (`random.Random.sample` boundaries, seeds
    `20261120` and `47424`), and the splitter re-splits every case at fixed chunk sizes 1, 2, 3, 5,
    7, 30, 40, 50, 100, 1000 — so a split landing inside a `\uXXXX` escape is pinned identically
    forever.
  - **Per-chunk state is recorded, not just the joined text**: `reply` / `done` / `aborted`,
    per-`feed` sentence groups, `flush`, a second `flush` (which must be empty), and `same_as_whole`
    per sequence.
  - **`app.logger.warning` → `tracing::warn!`**, and the fixture carries the message text so the
    Rust call can be checked to carry the same facts.
  - **`test_glm4.py` / `test_glm4_followup.py` are live-engine scripts**, not fixture modules; the
    one usable artefact is their document shape, which is in `document.json`.
  - **The fixture loader is shared** — `crates/shed-chat/tests/common/mod.rs` (`fixture`, `cases`,
    `field`, `value`, `input_text`, `expected_bool`, `expected_text`, `text_list`,
    `opt_text_list`, `chunks`). The earlier note saying "one `fn load()` per file" is superseded:
    `mod common;` per test binary is one parse per binary and it removed a dozen copies.
  - **Test naming**: the reference name verbatim minus `test_`, in one flat file per module; a name
    that collides across the reference's classes takes that class as a prefix; fixture-driven tests
    are `fixture_<section>` with no crate or module prefix.
  - **Python `None` may be fed to the Rust `&str` / `&[Value]` signatures as `""` / `&[]`** —
    verified live for all 14 quality functions and both stream helpers, because every one of them
    falsiness-tests or `str()`s its argument first.
  - `tools/capture_reply_fixtures.py` is the **authoritative definition of every fixture field**;
    re-running it is diff-clean (`json.dumps(indent=2, ensure_ascii=False)`), so it is also the way
    to answer "does the reference really do that" without booting anything.

  **Documented rather than fixed** (all four are rows in `docs/PARITY.md`):
  - A Python **set inside a reply document** serialises as a JSON array here. `Value` has no
    unserialisable member, so the literal parser renders it as an array; it cannot even be a
    fixture, because `json.dumps` raises `TypeError: Object of type set is not JSON serializable`.
  - The regex `\s` gap: Rust `\s` does not match U+001C–U+001F, Python's does.
  - `py_digit` implements `str.isdigit`, not `is_numeric`; they differ only for characters like `½`.
  - The `shed-gates::guards` catalogue-honesty check is **deferred, not written**. The catalogue's
    intro says the check is due now that 4f-1 has ported the first runtime guards, and its table row
    stays `pending (first guard)` — that is the honest state, and the check itself belongs with the
    slice that owns the gate binary.

  **Watch-outs that stayed true:** `too-many-lines-threshold = 50` counts *code* lines, so
  `normalize_reply_text` and `SentenceSplitter::feed` are split into helpers rather than given an
  `#[allow]`; Python `[:n]` slices count **code points** — decisive in the scanner's surrogate-pair
  escape handling, where the `char` is one code point but the *escape* is 12 bytes of input;
  `rustfmt.toml` is `max_width = 100`. Shell notes for this box: run cargo through
  `cmd /c "cargo … 2>&1"` and confirm the exit status with a trailing `&& echo OK || echo FAILED`;
  PowerShell mangles `$_` and nested quotes inside `cmd /c`, so `findstr` takes only single-word
  patterns and anything with a space goes through a redirected file; multi-line commit messages need
  `git commit -F <file>`; `> file` from PowerShell writes UTF-16.

- **SLICE 4e is COMMITTED and pushed** (`dc74524`, on `main`, 2026-11-20): `src/housekeeping.rs`
  (1448 lines — the strict record loader, the two transcripts, the summary and profile calls
  behind one `BackgroundModel` seam, `profile_work_for`, `ProfileWorkers`, and the
  `HousekeepingSettings` / `HousekeepingState` / `Housekeeper` trio that keeps the sweep off a
  bare thread), `tests/housekeeping.rs` (1565 lines, **all 25** of
  `test_conversation_housekeeping`: 21 sweep and profile tests in four batches, plus the four
  `UserProfileRequestTests`), `store.rs` (`ConversationRoot::record_files` and
  `CHARACTER_OUTLINE_FILENAME`, so the walk cannot mistake an outline for a conversation) and the
  `lib.rs` exports. Gate green: fmt, clippy `-D warnings`, **493 Rust passed / 0 failed**, 250 JS.
  **Next is 4f-1**: `TODO/SLICE-4f1-BRIEF.md` (written 2026-11-20, source pasted and verified byte-identical: `utils/reply.py` 389 lines + `agent.py:5504-6202` 699 lines, 136 tests listed). `TODO/SLICE-4f-BRIEF.md` is the scoping brief for the split.
  The commit carries the implementation as well as its tests: the 15 November session wrote
  `housekeeping.rs`, left it untracked, and committed nothing, so 4e lands as one commit and not
  two. `How_To_Code.md` carries an unrelated operator edit (principle 5's wording) and was
  deliberately left out of it, still uncommitted.
  What the tests had to work out, in case 4f needs the same: `to_system_time` clamps
  `timestamp().max(0)` while still adding `timestamp_subsec_nanos()`, so a negative moment lands
  ~0.5 s off and the cooldown fixture uses whole-second offsets; `HousekeepingSettings::default()`
  has `max_messages: 240`, which trims nothing, so every sweep test states `max_messages: 10`;
  the fake's default profile answer must be `{"user_details": {}}`, because a bare `{}` is
  `Err(ModelError::NotJson)`; mtime fixtures go through `OpenOptions::new().write(true).open(p)`
  plus `File::set_modified`; and `schedule_conversation_profile_update` returns `()`, so the
  "never raises into a turn" half is pinned by the type and only the observable half — a legacy
  key scheduling nothing — is pinned by an assertion.
  Two things batch 4 needed that are not obvious. `Arc::clone` into an `Arc<dyn Trait>` is **not**
  a coercion site, so handing a concrete fake to a scheduler that stores a trait object wants a
  `-> Arc<dyn BackgroundModel>` helper rather than an annotation at the call. And a worker with
  nothing to do finishes before an assertion can look for it, so the fake that proves a worker
  *was* started parks itself on a channel inside the model call — otherwise `workers.is_empty()`
  passes for a scheduler that could never start one either.

- **SLICE 4d is COMMITTED and pushed** (`20c420c`, on `main`, 2026-11-15): `src/store.rs`
  (`ConversationRoot`, `record_path`, `record_for_key`, the forgiving reader, the atomic writer with
  `Backup::Skip`, delivery style, `client_ids_seen_from_address`, `claim_legacy_conversation`) and
  `src/history.rs` (the profile and summary texts, `conversation_history` with the
  `PromptCachePlan` seam), plus `tests/store.rs` (the eight `test_conversation_identity` ports).
  Gate green: fmt, clippy `-D warnings`, **463 Rust passed / 0 failed**, npm. 30 new tests.
  `shed-store::atomic_write_json` has its first production caller. **Next is 4e**
  (`TODO/SLICE-4e-BRIEF.md`), then 4f's split.
  Note for a resuming session: the end of 4d was finished by hand — the model's `files__edit_file`
  call for the test fixture was refused (undeclared name), so the third client's address, a clippy
  `manual_unwrap_or_default` and `cargo fmt` were applied outside the model.

- **SLICE 4c is COMMITTED and pushed** (`e020573`, on `main`, 2026-11-14): `profile.rs` (`UserDetails` as
  an ordered pair list, the four `USER_DETAILS_*` constants, `detail_value_text`, `detail_label`,
  `take_chars`, `normalize_user_details`) and `compaction.rs` (`plan_conversation_compaction` with
  `cutoff` / `aged_out_prefix` / `is_worth_a_summary` / `retained_summaries`), 20 tests, with
  `record.rs`, `lib.rs`, `tests/records.rs`, both `Cargo.toml`s and `docs/PARITY.md` wired to them.
  Gate green on the committed tree: `fmt`, clippy `-D warnings`, **433 Rust passed / 0 failed**, npm.
  (Next is **4d** — conversation record I/O, `agent.py:3785-4174` — and it has no brief yet: 4d needs
  `TODO/SLICE-4d-BRIEF.md` written, with the source pasted inline, before its turn.)
  The bullet below is the pre-commit state, kept for the record.

- *(Historical.)* **SLICE-4c was IN PROGRESS, uncommitted, and nothing new had been compiled yet.** Two new files are
  on disk and **not yet declared in `lib.rs`**, so the crate still builds exactly as it did at
  `9becb98` (verified: 48 lib tests + 1 doc test, no warnings) and neither new file has been through
  rustc or rustfmt once. `crates/shed-conversation/src/profile.rs` holds `UserDetails` (an ordered
  `Vec<(String, String)>` with hand-written `Serialize`/`Deserialize` that keep the document's order
  and resolve a repeated key to *last value at the first position*), the four `USER_DETAILS_*`
  constants, `detail_value_text` / `detail_label` / `take_chars` / `normalize_user_details` and 10
  unit tests; `crates/shed-conversation/src/compaction.rs` holds `plan_conversation_compaction`
  beside `cutoff` / `aged_out_prefix` / `is_worth_a_summary` / `retained_summaries` and 10 unit
  tests — 20 in total, which is the brief's count. **Still to do, in this order:** (1) `record.rs` —
  `user_details` becomes `Option<UserDetails>` (drop `use std::collections::BTreeMap`), the "Two
  things this file deliberately does not decide" section is rewritten because 4c has now taken that
  decision, and `ConversationCompactionPlan`'s doc still says "slice 4c decides" what to put in it;
  (2) `lib.rs` — `mod compaction; mod profile;` plus re-exports, and the "What is NOT in here yet"
  paragraph, which currently claims the profile facts and the compaction plan are 4c's and that the
  record's shape is the loader's problem; (3) `tests/records.rs` — its `BTreeMap` fixture and the
  `empty_defaults` assertion that builds one; (4) `crates/shed-conversation/Cargo.toml` — `serde_json`
  moves from `[dev-dependencies]` to `[dependencies]` (the workspace list already has it) and the
  comment above it, "Only the dev test binary needs serde_json", becomes false; (5) `docs/PARITY.md`
  — the two 4c rows described below; (6) the gate, then commit
  `Stage 4 (part 2c): shed-conversation profile facts and the compaction plan`.
  **Two decisions already taken, not to be re-litigated on a resume:** `max_summaries` is a required
  argument rather than the source's `= 36` default (Rust has no keyword arguments and the housekeeper
  states its setting anyway — the brief allows either), and `serde_json`'s `BTreeMap` object is
  documented as an asymmetry rather than worked around, because turning on `preserve_order` swaps the
  map type for every `Value` in the workspace to save one line of a rendered prompt. The asymmetry is
  only on the way *in*: a profile parsed from a model reply leaves `normalize_user_details` in key
  order, while a profile loaded from a file keeps that file's order, which is the half `UserDetails`
  exists for and is pinned by `profile.rs`'s `a_profile_keeps_the_documents_order_and_a_duplicate_once`
  on stored bytes rather than on a parsed `Value`.
  **Watch:** `too-many-lines-threshold = 50` in `clippy.toml` with `too_many_lines` denied
  workspace-wide — `plan_conversation_compaction` is at 45 lines, so growth needs another helper, not
  an `#[allow]`. And `cmd /c` quoting: nested double quotes inside `cmd /c "… | findstr /c:"x" …"` get
  stripped by this harness, so run the gate steps with the tool's `cwd` and PowerShell's
  `Select-String -Pattern 'a|b'`, and check exit codes on an unpiped run.

- **Stage 4 part 2b is committed** (`122456f`): `timestamp.rs` (the fromisoformat ladder, the
  attach-not-convert comparison) and `freshness.rs` (unread rows, newest stamp as written, the
  return gap). 17 tests; gate green (fmt, clippy `-D warnings`, 409 Rust, npm). `chrono` is now a
  workspace dependency, added with its first consumer. **The model wrote both modules but never
  declared them in `lib.rs`, wrote no tests, and never added the dependency** — it wedged at
  0.1 tok/s (636 tokens in 3 h, GPU 1%, 21 W) because the host is out of RAM, see the next bullet.
- **Host RAM is the bottleneck, not the GPU.** 3.9 GB free of 62.8 GB; `--resident-budget-gib 40`
  plus `--conversation-cache-mib 4096` on a 63 GB box, with pagefiles peaking at 32 GB. The engine
  log shows 4.5 GiB of KV read from RAM per request and 3.7 TB read from the GGUF "in place".
  Turns crawl at 0.1 tok/s with the card idle. Lower the resident budget (24 GiB) and the
  conversation cache (1 GiB) before the next long run.

- **Stage 4 part 2a is committed** (`f98b9cc`): `crates/shed-conversation` — records, address,
  identity, filename, window. 31 ported tests + the absent-optional round-trip; gate green
  (fmt, clippy `-D warnings`, 392 Rust + 250 JS). **The model did not commit it and did not run
  the gate** — both were done by hand afterwards, and the gate passed first time.
- **`TODO/SLICE-C21E2-BRIEF.md` is fabricated. Delete it.** It describes `_INLINE_SEARCH_*`,
  `_search_root`, `_is_searchable` and a `search.rs`, and none of those exist in the source or
  anywhere in this repo. Any slice id outside 4a-4f is invented; check the plan's section 8.2
  before believing a slice exists.
- **The UI conversation that produced 2a hit the 94k context wall** and cannot be resumed:
  "prompt (94617 tokens) leaves no room to answer". Start a new chat per slice; the repo is the
  memory, not the conversation.
- **Stage 4 is open, part 1 committed and pushed.** `main` = `17419e6`, in sync with
  `origin/main`. The code commit is `294cea4` "Stage 4 (part 1): shed-store, the atomic
  document writer and the client id"; everything after it is documentation (`c349aee` plan
  slicing, `2a45c0f` How_To_Code principles 21/26, `17419e6` plan following the rename rule).
  Stages 0–3 done and gate-verified (`ceeac65`).
- Test totals at `294cea4`: **360 Rust + 250 JS** (Stage 3 closed at 344 + 250).
  Gate green at the commit: fmt / `clippy --workspace --all-targets -D warnings`
  / cargo test / npm test.
- **Nothing has been written for part 2 yet.** Since the commit the session did
  reconnaissance only (below), and stopped on request.
- **Recorded, not yet read into a plan:** the 5h31m stall of 2026-11-12 — last tool call
  11:55:42, no tool call and no file write until 17:26, while generating at ~30 t/s. There is
  no harness log beyond `%LOCALAPPDATA%\Strata\terminal\commands.log`; the plan's §8.2 slices
  and principle 26 exist because of it.
- Working tree carries **no code edits**. `How_To_Code.md` and `TODO/PLAN-Rust-Clone.md` are
  committed (2026-11-12). Untracked: this file, as at the Stage-3 snapshot.

## What Stage 4 part 1 landed
`crates/shed-store/` — `json_doc.rs` (`atomic_write_json`, `backup_path`,
`Backup::{Keep,Skip}`, `entity_store.py:46-101`) and `client_id.rs`
(`normalize`/`mint`/`resolve_client_id`, 230-248). 16 tests:
`tests/document_writer.rs` (10) + `tests/client_identity.rs` (6).

- **Nothing calls it yet.** First production caller is
  `write_conversation_record_file` (`agent.py:3910`) in `shed-conversation`,
  i.e. part 2. `pub` items in a lib are not `dead_code`, so the deny did not
  fire and no stub caller was needed.
- Two gate fixes on the inherited working tree: `clippy::manual_inspect` on the
  temp-file cleanup (now `inspect_err`; the error passes through untouched), and
  `cargo fmt` over the new crate. **The previous session had committed nothing
  because its gate had never been run** — fmt and clippy both failed on arrival.
- **Deliberate partial port of `test_entity_store_utils`** (15 tests): the 2 in
  `AtomicJsonWriteTests` port now; the 13 in `JsonlLogTests` are **deferred to
  Stage 7 with their functions** (`atomic_write_lines` 104, `read_jsonl_tail`
  142, `append_jsonl` 170 — their only callers are the thought journal, facts
  and events at 9717-10049, i.e. `shed-mind`). The quarantine recovery loader
  (193-228) and `client_file_path` (251) wait for Stage 7 for the same reason.
  Consistent with plan §8.1 ("`shed-store` at 4 for conversation files, at 7 for
  mind/client/outbox files"); plan §9's row still reads "4" for the whole suite,
  the one place the split is not written down — fix the §9 row when the plan is
  next edited (blocked behind decision 2 below).
- `client_file_path` hashes with `hashlib.sha1`, which the workspace does not
  carry (`sha2` only). Stage 7 decides: add `sha1` or digest with `sha2`. The
  digest is a filename inside the clone's own storage root, never a wire value.
- PARITY gained the row the crate doc promises: every stored JSON document is LF
  where the source's text-mode writes make it CRLF on Windows (same reasoning as
  the Stage-2 `models_cache.json` row; D9 keeps the two apps' files apart).

## Verified this session (so it does not get re-litigated)
- **The plan's line counts are correct; nothing has drifted.** `agent.py` =
  11,607; `utils/conversation_history.py` = **445**; `entity_store.py` 259;
  `text.py` 552; `reply.py` 389; `mind_state.py` 453. I briefly believed
  `conversation_history.py` was 368 lines and the plan was stale — **false
  alarm**, caused by my own measurement (see standing rules). The reference app
  has not moved, so the plan's region anchors (§2.1) are still trustworthy.
- `agent.py` and most utils are CRLF; `conversation_history.py` is LF. Irrelevant
  to the port (read as text), but it explains why a raw newline count differs
  per file if anyone re-counts.

## Stage 4 part 2 — reconnaissance done, no code yet
Read `utils/conversation_history.py` in full (445 lines). Inventory, for
`crates/shed-conversation/`:

| Lines | Function | Port notes / open questions |
|---|---|---|
| 42–90 | `ConversationMessage`, `ConversationSummary`, `ConversationRecord`, `ConversationCompactionPlan` | typed records; `delivery_style`, `user_details`, `user_details_covered_through` are `NotRequired` → `Option<String>` and a field that may be absent from the JSON **and** must be absent (not `null`) when written back. Records written before those fields existed must load |
| 93–112 | `_normalized_ip` | `std::net::IpAddr` covers the parse and `ipv4_mapped` (`Ipv6Addr::to_ipv4_mapped`). Bracket-stripping and `%scope`-stripping are hand-written before the parse. Python's `str(value or "")` falsy idiom is `shed_text`'s, at the call site |
| 115–129 | `resolve_client_ip` | Cloudflare header → first `X-Forwarded-For` entry → socket peer, in that order |
| 132–203 | `conversation_client_id`, `conversation_identity`, `conversation_filename`, `names_a_conversation` | these **call `shed_store::normalize_client_id`**, which is why part 1 landed first. Layer map already allows `shed-conversation → shed-store` ✓. `conversation_filename` **raises `ValueError`** → a real error type; it is the guard keeping attacker-controlled header text out of a path, so the rejection must stay a rejection, not a sanitise. IPv6 names store `:` as `-`, and `names_a_conversation` tries both spellings |
| 206–216 | `message_window_start_index` | `max_messages <= 0` → `len(messages)` (cap of zero trims all); a cap larger than the log returns 0 |
| 219–239 | `_parse_timestamp`, `_is_at_or_after` | **the main spike of part 2 — see below** |
| 242–309 | `unprofiled_messages`, `newest_message_timestamp`, `conversation_reopened` | unreadable stamps are **kept/counted as recent** in every case (one bad row must not delete a memory); `newest_message_timestamp` returns the **original spelling**, not a re-serialised datetime; `quiet_seconds <= 0` disables the greeting |
| 314–372 | `USER_DETAILS_*` consts, `_detail_value_text`, `_detail_label`, `normalize_user_details` | needs Python `str()` over numbers (shed-text, §9 "shed-text at 4"); `bool` is excluded from the numeric branch even though Python's `bool` **is** an `int`; `" ".join(value.split())` = collapse all whitespace; `other` is reserved, exempt from the 24-fact cap, allowed 512 chars vs 200; **dict iteration is insertion order**, and the first `other` wins via `break`; `[:200]`/`[:512]` slice by **code points**, not bytes — `chars().take(n)`, never string indexing |
| 375–445 | `plan_conversation_compaction` | two prefix cuts (age, message count), the **later** wins; overflow from the message cap is *always* summarised even with summary memory off, age-trimmed material only inside the summary window; `consumed_count` is carried so a turn landing mid-summary is not dropped. **My read was truncated at ~line 428 — re-read 428–445 before porting** (the `retained_summaries` tail and the `max_summaries` cap) |

### The timestamp spike (decide before writing the date code)
`datetime.fromisoformat` under Python 3.13 accepts a wide grammar: `T` **or
space** separator, optional fractional seconds of any length, `+HH:MM` /
`+HHMM` / `+HH` offsets, trailing `Z`, and naive values. `chrono`'s
`DateTime::parse_from_rfc3339` accepts far less, so a hand-written
`fromisoformat` equivalent is needed (chrono as a *dependency* is fine for
`Duration`/`Utc`; the permissive parse is the part to write and pin with tests).
Two traps:
- **Naive vs aware.** The source patches `cutoff`'s tz onto a naive parsed value
  (`_is_at_or_after` 237, `unprofiled_messages` 265, `conversation_reopened` 307)
  but **not** the reverse — an aware stamp against a naive cutoff raises
  `TypeError` in Python. Rust must not panic where Python raises; decide the
  clone's answer for that case and put it in PARITY if it is reachable.
- `total_seconds()` comparison in `conversation_reopened` is `>=`, and the
  subtraction direction matters for a negative gap.
`chrono` is **not** yet in `[workspace.dependencies]` — adding it needs a comment
naming its consumer (principle 16) and a resolution check, per §10 risk 10.

Also still to read before/while porting: `agent.py:3785–4174` (the record paths,
load/save, delivery style, history assembly — the I/O half that calls this
pure module), `agent.py:4185–4526` (housekeeping + profile workers),
`agent.py:4529–5000` (recall + `enforce_persona_reply` @4886), and the three
Python suites to port: `test_conversation_history_utils`,
`test_conversation_housekeeping`, `test_conversation_identity`.

## Next move (Stage 4 part 2, then the rest of Stage 4)

**Stage 4 is sliced in the plan's §8.2: one slice per turn, commit-or-stop.** Start the next
turn from `TODO/SLICE-4a-BRIEF.md`, which carries the source inline and says which lines to
read. **Do not read `agent.py` for 4a** — the brief replaces that reading, and reading it is
what consumed the previous session.
`shed-conversation` is the crate that gives `shed-store` its first caller. Then
Stage 4 as a whole still owes: `shed-chat` (reply shaping, JSON reply scanner,
sentence splitter, stream publisher, `run_agent`, `TurnGate`), `POST /api/chat` +
the job route, the `chat_job_not_found` 500 envelope (`agent.py:9409` — the
envelope's **first reachable caller**, plan §8.1), `test_api_chat` +
`test_reply_*` (4) + `test_sentence_splitter` + `test_glm4*` + `test_text_utils`
(reply parts), **golden fixtures captured from the live Python app** (chat turns,
glm4 output, job `sentences[]` progress — D8, must happen before that app is
retired), and the exit gates: a chat turn streaming sentences to the frozen
client, and the busy path reproducing under two simultaneous requests.
GUARD_CATALOGUE's six Stage-4 rows are all still `pending`; the first ported
runtime guard (`TurnGate`, persona enforcement, job TTL/cap, duplicate-event,
reply recovery) turns one **ported** and is what the `shed-gates::guards` honesty
gate waits for.

## Housekeeping owed
- At stage close, with the operator's say-so, delete leftover Stage-2
  export/resume notes under `D:\temp` (plan §8.1). The plan file itself is
  renamed `_DONE` only at Stage 9.

## Decisions owed the operator (both pre-date this session; neither is mine to make)

**Both are now resolved** (2026-11-12, operator's instruction) — do not re-litigate them:

1. **Resolved.** `How_To_Code.md` is edited and committed: "Functions over ~50 lines" stands,
   principle 21 now says a finished plan is renamed `_DONE` rather than deleted, and a new
   principle 26 states the slice/commit rule.
2. **Resolved.** The pasted block is out of the plan and lives in `docs/LATENCY-NOTES.md`,
   tidied; the Stage 6 row points at it. Nothing was dropped.

## Standing rules (do not relearn the hard way)
- **Counting lines: never `Get-Content f | Measure-Object -Line` — it silently
  drops blank lines** (that is how 445 became 368 and cost a false "the plan is
  stale" alarm). Count newline characters in the raw text instead:
  `([regex]::Matches([IO.File]::ReadAllText($full),"`n")).Count`. And note .NET
  APIs **ignore PowerShell's current directory** — pass absolute paths, or they
  resolve against `D:\`.
- **Gate runs: use `cmd /c "cargo … 2>&1"`.** PowerShell turns any stderr line
  into a `NativeCommandError` display artefact and cargo writes progress to
  stderr, so a green run can *look* failed and `2>&1 | …` can even leave
  `$LASTEXITCODE = 1` on a passing test run. `cmd /c` gives merged output and a
  reliable exit code. Tally passes with:
  `foreach($l in $out){ if($l -match "(\d+) passed"){ $s += [int]$Matches[1] } }`.
- **Never restore a source file with `Copy-Item` from a backup.** The copy keeps the **backup's**
  LastWriteTime, which is older than the build made from the edited file, so cargo judges the
  target fresh and re-runs the **stale** binary: correct source on disk, phantom failures from
  the previous content, and a green `--workspace` run in between that convinces you it is
  flaky. Undo an edit with the files tool; if you must restore out of band, set
  `(Get-Item $p).LastWriteTime = Get-Date` afterwards and check for a `Compiling` line before
  believing any count.
- **`too-many-lines-threshold = 50` counts code lines, not raw ones** — comments and blanks are
  free. A comment-heavy test can sit at 63 raw lines and pass, and sit at 49 code lines and be
  one assertion from failing. Measure it, don't eyeball it:
  `($lines[$s..$e] | Where-Object { $_.Trim() -ne '' -and -not $_.Trim().StartsWith('//') }).Count`.
- NEVER boot the reference Python app or bind port 5005 — the production service
  owns it and Werkzeug joins silently (the clone refuses instead; see the PARITY
  row on os error 10048). Clone runs use `--base-dir <path>` and a spare
  `APP_PORT` (the clone repo has no `.env`, so defaults would hit 5005).
- Clone = `D:\Entity_In_The_Shed` (underscore). Python reference =
  `D:\Entity-In-The-Shed` (hyphen). Server binary is `entity_in_the_shed.exe`.
- PowerShell: `;` not `&&`.
- Workspace lints deny dead_code/unused/unwrap_used/expect_used etc. — unused
  deps/items are hard errors; test binaries need `use crate as _;` stubs.
- API bytes must match Flask exactly: compact JSON, sorted keys, trailing
  newline, no charset, `\uXXXX` escapes (shed-http `json.rs::EnsureAscii`), Jinja
  text-mode page read (CRLF→LF, drop one trailing newline, `page.rs`).
- `rustfmt.toml` is `max_width = 100` only, so rustfmt's default
  `struct_variant_width = 35` breaks two-field enum variants onto several lines.
  Run `cargo fmt --all` after writing any enum with struct variants.
- **Run the stage gate before committing, not after** — part 1 arrived with fmt
  and clippy both red.
- Python slice semantics in ports: `[:n]` counts **code points**; and Python
  `bool` is an `int`, which the source explicitly excludes in
  `_detail_value_text`.
- Known frozen-catalogue artefacts (NOT bugs): live `/api/voices` answers 333
  voices vs frozen 323 (Edge provider also enumerates Windows voices,
  `agent.py:3232`); stale `fil-PH-BlessicaNeural` row; lost tie order. All in
  PARITY + plan §8.1, removed at Stage 6.
- D9: a held port stops the clone at bind (os error 10048, exit 1) — intended.

## Key files
- `TODO/PLAN-Rust-Clone.md` — source of truth for stages (§8 ~line 336, §8.1).
- `docs/PARITY.md`, `docs/GUARD_CATALOGUE.md` — current as of `294cea4`.
- `crates/shed-store/` — atomic JSON document writer + client id; **no caller
  yet**, part 2 installs it. JSONL log + recovery loader are Stage 7.
- `crates/shed-http/` — json.rs, page.rs, state.rs/lib.rs, api/models.rs.
- `crates/shed-tts/src/catalog.rs` — frozen snapshot; Stage 6 installs live fetchers.
- `bins/shed-server/src/main.rs` — boot order, `--smoke`, `base_dir(args)`.
- `crates/shed-config/src/config.rs` — `AppConfig::load_from`, DEFAULT_STORAGE_ROOT.
- Reference snapshot kept: `%TEMP%\opencode\agent_at_7864f77.py`.

