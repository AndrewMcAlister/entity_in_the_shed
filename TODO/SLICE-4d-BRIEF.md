# Slice 4d brief — the conversation record on disk, and the history the model sees

Hand this file to the session as its task. The source is pasted below and the decisions are
already made. **Do not read `agent.py`.** Everything this slice ports is between the two
`# ---- source ----` markers below; the rest of the file is not your reading list.

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`, so a bare `TODO/...` resolves to `D:\TODO\...` and
fails: pass the full `D:\Entity_In_The_Shed\...` path to the files tool, and start every terminal
command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2, where this slice is already written down at **line 375**:
  `| **4d** | Conversation record I/O: paths, load/save, delivery style, history assembly, legacy address-record adoption | agent.py:3785-4174 | …|`. You do not need to search for it, and if a `findstr` comes back empty the search was wrong, not the plan.
- **Do not spend a round confirming the slice exists.** There is no other copy of this task to
  find: the brief in your hands is it.
- Rules: `How_To_Code.md`. Workspace lints deny `dead_code`, `unused`,
  `unused_crate_dependencies`, `clippy::unwrap_used`, `clippy::expect_used` **including test
  targets**.
- The crate already has `address.rs`, `filename.rs`, `freshness.rs`, `identity.rs`, `profile.rs`,
  `compaction.rs`, `record.rs`, `timestamp.rs`, `window.rs` (4a–4c). Read `record.rs`,
  `filename.rs` and `freshness.rs` before writing anything: this slice is mostly their callers.
- `main` is the branch here, not `develop`. 4c committed as `e020573`.

## Mission

Five things, and nothing else:

1. **Where a record lives** — the filename from a conversation key, under the memory root or a
   character's folder. `shed-store`'s writer has no caller yet; this slice is its first.
2. **Reading it back** — a damaged or missing file reads as the empty record, never as an error.
3. **Writing it** — atomically, and **without** a `.bak` (`Backup::Skip`).
4. **Which style the last turn used** — so continuing a conversation continues the way it was
   spoken, until the record comes back with none.
5. **The history the model sees** — profile text, then summary text, then the exact tail, with the
   message cap applied and the prompt-block seam honoured.

## The source, verbatim (1) `agent.py` lines 3785–3930

```python
def conversation_record_path(conversation_key, character_profile=None):
    """Where one conversation lives: the character's folder, or the root.

    The name is the browser id when the client sends one, or its address when
    it does not, so the file that holds a returning visitor's conversation is
    recognisable without a lookup table.
    """
    filename = conversation_filename(conversation_key)
    if not character_profile:
        return MEMORY_DIR / filename

    folder = conversation_folder(character_profile)
    return folder / filename


def conversation_record_for_key(conversation_key):
    """The record holding one identity's conversation, or ``None``.

    The same identity can hold a conversation with more than one character, and
    each lives in that character's folder. The newest is the conversation as it
    stands: when a voice was changed mid-page, the later one is the one being
    had, and the earlier one is history.
    """
    conversation_key = str(conversation_key or "").strip()
    if not conversation_key:
        return None
    filename = conversation_filename(conversation_key)
    matches = list(MEMORY_DIR.glob(f"*/{filename}"))
    root_match = MEMORY_DIR / filename
    if root_match.exists():
        matches.append(root_match)
    if not matches:
        return None
    return max(matches, key=lambda path: path.stat().st_mtime)


def client_ids_seen_from_address(client_ip) -> set:
    """Every browser id already recorded against one address."""
    wanted = str(client_ip or "").strip()
    if not wanted:
        return set()
    return {
        client_id
        for client_id, record in iter_entity_client_records()
        if str(record.get("client_ip") or "").strip() == wanted
    }


def claim_legacy_conversation(conversation_key, client_ip, character_profile=None) -> None:
    """Move an address-keyed transcript to the one browser that owns it.

    Before conversations were keyed by browser, a record was named for the
    address. The first browser of an address to come back after the change
    inherits that transcript rather than appearing to have lost it. Only a
    browser not known to share the address can inherit: once two ids have been
    recorded there, there is no deterministic answer to whose words those were,
    so neither takes them and each starts its own history.
    """
    client_id = conversation_client_id(conversation_key)
    address_key = conversation_identity("", client_ip)
    if not client_id or not address_key:
        return

    target = conversation_record_path(conversation_key, character_profile)
    legacy = conversation_record_path(address_key, character_profile)
    with CONVERSATION_LOCK:
        if target.exists() or not legacy.exists():
            return
        if not client_ids_seen_from_address(address_key) <= {client_id}:
            return
        try:
            os.replace(str(legacy), str(target))
        except OSError as exc:
            app.logger.warning(f"Could not adopt legacy conversation {legacy}: {exc}")


def parse_timestamp(value):
    return datetime.fromisoformat(value)


def default_conversation_record():
    return {"messages": [], "summaries": []}


def read_conversation_record_file(file_path):
    """One conversation record off disk, or the empty record.

    A record that cannot be read is read as an absence rather than raising. The
    turn reads these inside its own lock and the slow loop reads one with nobody
    watching, and neither has anything better to do about a damaged file than
    carry on without it -- an empty record being the honest reading of a
    conversation nobody can read.
    """
    if not file_path or not file_path.exists():
        return default_conversation_record()

    try:
        return json.loads(file_path.read_text(encoding="utf-8"))
    except Exception:
        return default_conversation_record()


def load_conversation_record(conversation_key, character_profile=None):
    if not conversation_key:
        return default_conversation_record()

    return read_conversation_record_file(
        conversation_record_path(conversation_key, character_profile=character_profile)
    )


def conversation_delivery_style(conversation_key, character_profile=None):
    """The style this conversation was last delivered in, or ``""``.

    Read before a turn resolves its own style: with no explicit choice on the
    request, continuing the conversation means continuing the way it was being
    spoken. An empty answer leaves the decision to the person's wider habits.
    """
    if not conversation_key:
        return ""

    record = load_conversation_record(conversation_key, character_profile=character_profile)
    return str(record.get("delivery_style") or "").strip()


def write_conversation_record_file(file_path, record) -> None:
    """Store a conversation record atomically.

    Two writers now share this file -- the turn and the profile worker -- and
    a reader that catches half a write would read a truncated conversation.
    No ``.bak`` is kept: a transcript is written every turn, and a second copy
    of it beside each record is space spent to preserve words that are already
    in the record it was copied from.
    """
    atomic_write_json(file_path, record, keep_backup=False)


def save_conversation_record(conversation_key, record, character_profile=None):
    if not conversation_key:
        return
    write_conversation_record_file(
        conversation_record_path(conversation_key, character_profile=character_profile),
        record,
    )

```

## The source, verbatim (2) `agent.py` lines 4082–4174

```python
def build_user_details_system_text(user_details, messages):
    """What the conversation knows about the person, as one system message's text.

    Empty when nothing is known. The reunion line is added only when a name is
    known and the transcript shows a gap: the conversation outlives the visit, so
    the gap in the transcript can say what the page cannot.
    """
    details = normalize_user_details(user_details)
    if not details:
        return ""

    detail_lines = [
        f"- {label.replace('_', ' ')}: {value}"
        for label, value in sorted(
            details.items(),
            key=lambda item: (item[0] == USER_DETAILS_OTHER_LABEL, item[0]),
        )
    ]
    name = str(details.get("name") or "").strip()
    if name and conversation_reopened(
        messages,
        now=datetime.now().astimezone(),
        quiet_seconds=CONVERSATION_REUNION_SECONDS,
    ):
        detail_lines.append(
            f"They have just come back after a break. Greet them by their "
            f"name ({name}) before answering."
        )
    return "What this conversation knows about the person:\n" + "\n".join(detail_lines)


def build_summary_system_text(summaries):
    """The gist of older talk, as one system message's text; empty when none."""
    summary_lines = [
        f"- {item.get('summary')}"
        for item in (summaries or [])
        if isinstance(item, dict) and str(item.get("summary") or "").strip()
    ]
    if not summary_lines:
        return ""
    return "Conversation summary memory:\n" + "\n".join(summary_lines)


def get_conversation_history(conversation_key, character_profile=None, prompt_cache=None):
    """The conversation as the model should see it: profile, memory, exact tail.

    Read-only. Trimming is the idle housekeeper's work: a turn should not spend
    the request summarizing older material or rewriting a file another request
    may be appending to. The message cutoff is still applied here, so the
    prompt never carries more rows than the stored conversation keeps.

    The profile comes first because it is the most durable thing the
    conversation knows, and a person who has been away is greeted by the name it
    holds. Memory is the gist of older talk; the exact tail is last, because
    the newest words are the ones the reply has to answer.

    With ``prompt_cache`` set, the profile and the summary are held as one named
    block for this conversation and the history carries its placeholder in one
    system message instead. The block is stored by the caller before the engine
    is asked to expand it; without that cache the two texts are sent inline, as
    the two messages they have always been.
    """
    if not conversation_key:
        return []

    record = load_conversation_record(conversation_key, character_profile=character_profile)
    messages = list(record.get("messages") or [])
    exact_start = message_window_start_index(messages, CONVERSATION_MAX_MESSAGES)

    user_details_text = build_user_details_system_text(record.get("user_details"), messages)
    summary_text = build_summary_system_text(record.get("summaries"))

    history = []
    if prompt_cache is not None:
        user_info_text = "\n\n".join(part for part in (user_details_text, summary_text) if part)
        if user_info_text:
            block_key = user_info_block_key(
                conversation_key, character_prompt_cache_identity(character_profile)
            )
            history.append(
                {"role": "system", "content": prompt_cache.reference(block_key, user_info_text)}
            )
    else:
        for text in (user_details_text, summary_text):
            if text:
                history.append({"role": "system", "content": text})

    history.extend(
        {"role": item["role"], "content": item["content"]}
        for item in messages[exact_start:]
        if isinstance(item, dict)
    )
    return history
```

## What is deliberately NOT in this slice

- **`summarize_conversation_messages` (3931–4000) and `update_conversation_user_details`
  (4002–4079)** are out. Both are HTTP calls to the engine, and their only callers are the
  housekeeping and profile workers — which are slice **4e** (`agent.py:4185-4526`). Their prompts,
  their response parsing and their four tests (`UserProfileRequestTests`, `FakeJsonResponse`) go with
  those workers, not with record I/O. This is the same deliberate split Stage 4 part 1 made with
  `JsonlLogTests` → Stage 7: the function goes where its caller goes.
- **`ensure_character_outline`** (`agent.py:3490`-ish) is out: creating a character's folder and
  writing its outline file is `shed-characters`' work, and `atomic_write_json` creates the parent
  directory anyway. This slice takes the folder, it does not create it.
- **`parse_timestamp`** is out: `timestamp.rs` (4b) already owns the parse, and a one-line
  `fromisoformat` wrapper would be a second door into the same room.
- **`MEMORY_DIR`** is out: it is a module global in the source. Every function here takes the root
  as a value (`shed-config` will supply it later).

## Decisions already taken — do not re-derive

1. **Module split**: `src/store.rs` gets the record-on-disk half (`conversation_record_path`,
   `conversation_record_for_key`, `read_conversation_record_file`, `load_conversation_record`,
   `conversation_delivery_style`, `write_conversation_record_file`, `save_conversation_record`,
   `client_ids_seen_from_address`, `claim_legacy_conversation`) and `ConversationRoot`;
   `src/history.rs` gets the reading half (`build_user_details_system_text`,
   `build_summary_system_text`, `conversation_history`). Re-export both from `lib.rs`.
2. **The root is a value, and so is the folder.** `pub struct ConversationRoot { pub memory_dir: PathBuf }`
   with `pub fn record_path(&self, filename: &str, character_slug: Option<&str>) -> PathBuf`
   (`memory_dir / slug / filename` when a slug is given, else `memory_dir / filename`). The caller
   computes `filename` once through 4a's `conversation_filename(key)?` so the rejection stays a
   rejection at the boundary; `record_path` takes the validated `&str` and cannot fail. Do **not**
   take the character profile dict: the clone has `shed_characters` for that, and this crate's layer
   position (`shed-conversation → config, json, text, store, models, prompts`) does not include it —
   pass the slug.
3. **`record_for_key`** takes a filename and returns `io::Result<Option<PathBuf>>`: scan
   `memory_dir/*/<filename>` and `memory_dir/<filename>`, keep the newest `modified()`. Ties and
   unreadable metadata: sort by `(mtime, path)` and take the last, so the answer is deterministic
   where Python's `max` over a glob is not — add one `docs/PARITY.md` line for that.
4. **Reading is forgiving, writing is not.** `read_conversation_record_file(path) -> ConversationRecord`:
   missing file, unreadable file, invalid JSON → `ConversationRecord::default()` (add
   `#[derive(Default)]` to it). It returns the record, not a `Result` — the source's docstring is the
   reason, and a caller that has to handle an error would only ever return the same empty record.
   `write_conversation_record_file(path, &record) -> Result<(), WriteError>` passes
   `Backup::Skip`, which is what `keep_backup=False` means: no `.bak` beside a file written every turn.
5. **`load_conversation_record`** takes `Option<&str>` for the filename or the key: a missing or
   empty key is the empty record, not an error (the source's `if not conversation_key`).
6. **`conversation_delivery_style`** returns `String`, trimmed, `""` when the field is absent — not
   `Option<String>`. The source's `str(... or "").strip()` erases the difference between absent and
   empty, and every caller treats them the same.
7. **The legacy move is a function over data, not a trait.** `client_ids_seen_from_address` becomes a
   free function over client records:
   `pub fn client_ids_seen_from_address(clients: &[(String, String)], address: &str) -> BTreeSet<String>`
   (pairs of `client_id`, `client_ip`), and
   `pub fn claim_legacy_conversation(root: &ConversationRoot, filename: &str, legacy_filename: &str, clients: &[(String, String)]) -> Result<bool, io::Error>`
   where the bool means "adopted". The client-file store is Stage 7 (`shed-store`'s client files), so
   the caller supplies the pairs; nothing here reads a clients directory. The guard order is the
   source's and matters: target exists → no; legacy missing → no; any recorded id outside
   `{client_id}` → no; then `fs::rename`. A rename failure is returned, not logged — the clone's
   caller decides what to do with it.
8. **`build_user_details_system_text` takes values, not globals**:
   `(details: Option<&UserDetails>, messages: &[ConversationMessage], now: DateTime<Utc>, reunion_seconds: i64) -> String`.
   Reuse 4c's `normalize_user_details` and 4b's `conversation_reopened`; the sort key is
   `(label == USER_DETAILS_OTHER_LABEL, label)` — `other` last, everything else alphabetical — and the
   label's underscores become spaces on the way out, not in the stored profile.
9. **`build_summary_system_text(summaries: &[ConversationSummary]) -> String`**: skip a summary whose
   `summary` is empty or whitespace; one `- ` line each; `""` when none. The typed rows mean the
   source's `isinstance(item, dict)` disappears (already a PARITY row from 4b).
10. **History assembly is separate from loading.** `conversation_history(record: &ConversationRecord, conversation_key: &str, character_identity: &str, max_messages: i64, now: DateTime<Utc>, reunion_seconds: i64, prompt_cache: Option<&mut PromptCachePlan>) -> Vec<HistoryMessage>`.
    The source loads the record inside its own body; the clone's caller already holds it, and splitting
    the two is what makes the order testable without a temp directory. `HistoryMessage` is a new
    `{ pub role: String, pub content: String }` in `history.rs` — `ConversationMessage` carries a
    timestamp and is not this type.
11. **The prompt-block seam is `shed_prompts::blocks`.** `Some(cache)` → build `user_info_text` from the
    two non-empty texts joined by `"\n\n"`, then one system message whose content is
    `cache.reference(&user_info_block_key(conversation_key, character_identity), &user_info_text)`.
    `None` → the two texts as two separate system messages, profile first. `user_info_block_key` and
    `PromptCachePlan` already exist there — do not write a second copy. `character_identity` is the
    caller's `character_prompt_cache_identity` equivalent; pass `""` when there is no character.
12. **The cap applies to the tail only**, through 4a's
    `message_window_start_index(messages.len(), max_messages)`; a cap of 10 keeps the last ten rows.
    Rows are emitted in stored order, roles as stored, contents as stored — no re-rendering, no
    trimming of content.
13. **Constants are arguments.** `CONVERSATION_MAX_MESSAGES` (default 10) and
    `CONVERSATION_REUNION_SECONDS` (default 3600) are environment reads in the source; the clone's
    crates do not read env outside `shed-config`, so they arrive as `i64` parameters. Tests pass
    10 and 3600.
14. **Where the file goes and what it is called**: `ConversationRoot::record_path` is the only place
    that joins paths. `memory_dir` is the root the caller supplies (the source's
    `STORAGE_ROOT / "Conversations"`, `shed-config`'s job later).

## Tests to port

From `tests/test_conversation_identity.py` — **all eight**, in
`crates/shed-conversation/tests/store.rs` (a new integration test binary; the two `client_*`
fixtures become the `&[(String, String)]` pairs decision 7 asks for):

- `test_two_browsers_at_one_address_keep_separate_conversations`
- `test_a_browser_keeps_one_key_when_its_address_changes`
- `test_a_turn_without_a_browser_id_falls_back_to_its_address`
- `test_a_legacy_address_record_is_adopted_by_its_only_known_browser`
- `test_an_unclaimed_address_record_may_be_taken_by_the_first_browser`
- `test_a_shared_address_leaves_the_legacy_record_to_no_one`
- `test_a_browser_that_already_has_history_never_picks_up_the_legacy_record`
- `test_an_address_only_client_never_claims_a_legacy_record`

From `tests/test_conversation_housekeeping.py` — the record-path parts only, and only these two:

- `test_a_damaged_record_is_left_alone` (the reading half: a damaged file reads as the empty
  record, and is not overwritten by the read)
- `test_trimming_a_conversation_keeps_the_style_it_was_spoken_in` — port only the **style read**
  half (`conversation_delivery_style` returns the stored style, and `""` when absent). The trimming
  half of that test is 4e's.

Plus unit tests in `store.rs`/`history.rs` for the parts the Python tests leave implicit: the
default record round-trips as `{"messages":[],"summaries":[]}`; a record written before the three
optional fields existed loads and is written back **without them** (this is the slice's
`Done when`, and `tests/records.rs` already pins the same rule at the type level — pin it here at
the file level); `record_for_key` picks the newest of two character folders; a cap of 0 keeps no
tail rows; the profile sorts `other` last; the reunion line appears only with a name **and** a gap.

Read those test bodies: the fixtures pin exact JSON, the timestamps are fixed strings, and the
adoption tests choose who is "known" on purpose.

## Gate (must be green before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # 433 before this slice; report what you get
cmd /c "npm test 2>&1"                      # green, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Gate green** → commit `Stage 4 (part 2d): the conversation record on disk and the history the
   model sees`, push, add one line to `TODO/RESUME-NOTES.md` under "Where we are" (4d committed,
   with the hash), and **stop**. Do not start 4e.
2. **Gate red after two honest fix attempts**, or the source and this brief disagree → **stop** and
   report the failing output plus the disagreement. Do not widen the slice.

## Standing rules for this repo

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- `cargo fmt --all` after writing; the workspace is `max_width = 100`.
- A dependency you add to a crate must also be listed in the root `[workspace.dependencies]` with
  the crate that consumes it, or `cargo clippy` fails at the workspace level. `shed-prompts` is
  already a workspace dependency; `shed-conversation` may name it (layer map §3.3 allows prompts).
- PowerShell: `;` not `&&`. Absolute paths everywhere.
- Tool calls: use the tools' real names — `write_file`, `read_file`, `edit_file`, `grep_files`,
  `run_command` — never a prefixed spelling such as `files__write_file` or `terminal__run_command`
  and never `<function=NAME>` in the text. A call written that way goes back as text and costs the
  whole turn.
