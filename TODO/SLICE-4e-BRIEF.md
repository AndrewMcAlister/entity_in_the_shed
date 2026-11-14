# Slice 4e brief — the idle housekeeper and the profile workers

Hand this file to the session as its task. The source is pasted below and the decisions are
already made. **Do not read `agent.py`.** Everything this slice ports is inside the two
`# ---- source ----` fenced blocks; the rest of that file is not your reading list.

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`, so a bare `TODO/...` resolves to `D:\TODO\...` and
fails: pass the full `D:\Entity_In_The_Shed\...` path to the files tool, and start every terminal
command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2, **line 376**: `| **4e** | Housekeeping and profile workers |
  agent.py:4185-4526 | test_conversation_housekeeping (25, the rest) | the sweep is driven from a
  named owner, never a bare thread; a turn landing mid-summary is kept (consumed_count)`.
- **Do not spend a round confirming the slice exists.** The brief in your hands is it.
- Rules: `How_To_Code.md`. Workspace lints deny `dead_code`, `unused`,
  `unused_crate_dependencies`, `clippy::unwrap_used`, `clippy::expect_used` **including test
  targets**.
- 4d already landed (`src/store.rs`, `src/history.rs`): the record paths, the forgiving reader, the
  atomic writer with `Backup::Skip`, delivery style and history assembly exist. This slice is their
  callers, plus the two model calls 4d deliberately left out.
- `main` is the branch here.

## Mission

Three things:

1. **The profile worker** — fold what the person just said into their stored profile as each turn
   ends, off the turn's thread, one worker per conversation, and never let a failure reach the turn.
2. **The idle housekeeper** — one sweep over conversations that have gone quiet: bring an old
   record's profile up to date, trim and summarise past the caps, and remember what it examined so
   an untouched file is read once, not every thirty seconds.
3. **The two model calls** — `summarize_conversation_messages` and
   `update_conversation_user_details`: their prompts and their parsing are ported here; the
   transport is injected (decision 1), because this crate does not depend on `shed-llm`.

## The source, verbatim (1) `agent.py` lines 3931–4080 — the two model calls

```python
def summarize_conversation_messages(messages, character_profile=None):
    if not messages:
        return ""

    transcript_lines = []
    for message in messages:
        role = message.get("role", "user")
        content = str(message.get("content", "")).strip()
        if content:
            transcript_lines.append(f"{role}: {content}")

    transcript = "\n".join(transcript_lines)[-12000:]
    if not transcript:
        return ""

    response = call_lm_http_with_retry(
        "POST",
        f"{LM_ENGINE_HOST}/v1/chat/completions",
        timeout=LM_ENGINE_REPLY_TIMEOUT,
        purpose="conversation summary",
        json={
            "model": resolve_background_model(),
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "Summarize the older conversation below for a long-term memory log. "
                        "Return JSON only with the schema {\"summary\": \"...\"}. "
                        "Write one short, vague sentence of at most twenty words: the gist of "
                        "what was talked about, without names, numbers, decisions or details. "
                        "Durable facts about the person are kept in their profile, so do not "
                        "repeat them here. "
                        + (
                            f"This memory belongs to the character {character_profile['name']} ({character_profile['locale']})."
                            if character_profile
                            else ""
                        )
                    ),
                },
                {"role": "user", "content": transcript},
            ],
        },
    )
    if response.status_code >= 400:
        snippet = _safe_body_snippet(response.text)
        raise RuntimeError(
            f"LM summary request failed with status {response.status_code} and content_type "
            f"{response.headers.get('Content-Type', '')}: {snippet}"
        )
    payload = parse_json_response_strict(response, "LM summary response")
    choices = payload.get("choices", [])
    if choices:
        content = choices[0].get("message", {}).get("content", "") or "{}"
    else:
        content = "{}"
    content = str(content).strip()

    try:
        summary = json.loads(content).get("summary", "").strip()
    except Exception:
        summary = ""

    if summary:
        return summary

    # A model that answers in prose still leaves a trace, but the trace is one
    # line: this list is read on every later turn, so a fallback must not be
    # the door a whole transcript comes back through.
    return " ; ".join(transcript_lines[-2:])[:240]


def update_conversation_user_details(messages, current_details):
    """Ask the model to fold what the person just said into their profile.

    Returns the whole profile, not a patch: the model sees the profile as it
    stands and answers with the updated one, so a fact it is not reminded of is
    a fact that stays. An answer that cannot be read raises rather than
    returning the old profile, because the caller trims the messages this read
    was built from -- a silent failure there would drop what the person said
    rather than retry it.
    """
    transcript_lines = []
    for message in messages:
        if not isinstance(message, dict):
            continue
        content = str(message.get("content") or "").strip()
        if content:
            transcript_lines.append(f"{message.get('role') or 'user'}: {content}")

    transcript = "\n".join(transcript_lines)[-12000:]
    if not transcript:
        return dict(current_details or {})

    response = call_lm_http_with_retry(
        "POST",
        f"{LM_ENGINE_HOST}/v1/chat/completions",
        timeout=LM_ENGINE_REPLY_TIMEOUT,
        purpose="conversation user profile",
        json={
            "model": resolve_background_model(),
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "Maintain a short profile of the person this conversation is with. "
                        "Return JSON only with the schema {\"user_details\": {\"label\": \"value\"}}. "
                        "Record only what the person has said about themselves or their preferences: "
                        "name, age, likes and dislikes, the town or city they live in, country, "
                        "family, and anything else that could matter in a later conversation. "
                        "For example, if they mention a sick mother, add "
                        "\"family\": \"their mum is unwell\". "
                        "Keep every fact already in the profile unless the new messages correct it. "
                        "Use short lower-case labels; each value is a short phrase, except \"other\", "
                        "which is for anything that does not fit a label and may run to 512 characters. "
                        "Do not guess."
                    ),
                },
                {
                    "role": "user",
                    "content": (
                        "Current profile:\n"
                        + json.dumps(current_details or {}, ensure_ascii=False, sort_keys=True)
                        + "\n\nNew messages:\n"
                        + transcript
                    ),
                },
            ],
        },
    )
    if response.status_code >= 400:
        snippet = _safe_body_snippet(response.text)
        raise RuntimeError(
            f"LM user profile request failed with status {response.status_code} and content_type "
            f"{response.headers.get('Content-Type', '')}: {snippet}"
        )
    payload = parse_json_response_strict(response, "LM user profile response")
    choices = payload.get("choices", [])
    if choices:
        content = choices[0].get("message", {}).get("content", "") or "{}"
    else:
        content = "{}"

    try:
        details = json.loads(str(content).strip()).get("user_details")
    except (ValueError, AttributeError) as exc:
        raise RuntimeError("LM user profile response was not JSON") from exc
    if not isinstance(details, dict):
        raise RuntimeError("LM user profile response carried no user_details object")
    return normalize_user_details(details)
```

## The source, verbatim (2) `agent.py` lines 4185–4526 — the workers and the sweep

```python
CONVERSATION_HOUSEKEEPING_STATE_LOCK = threading.Lock()
# The mtime each record was last examined at. A file that needed nothing is not
# read again until something appends to it and the mtime moves.
CONVERSATION_HOUSEKEEPING_CHECKED_MTIME = {}
# Failures wait before retrying: a model that is down should not be asked every
# poll. In memory only -- a restart is as good a moment to retry as any.
CONVERSATION_HOUSEKEEPING_RETRY_AT = {}

# The profile is written as each turn ends rather than waiting for idle: it is
# what the next visit is greeted by, and a record that learns a name five
# quiet minutes later has been a stranger for the whole visit. One pass at a
# time, because there is one resident model; one worker per conversation, so
# several quick turns coalesce into the running pass rather than a queue.
CONVERSATION_PROFILE_LOCK = threading.Lock()
CONVERSATION_PROFILE_WORKERS_LOCK = threading.Lock()
CONVERSATION_PROFILE_WORKERS = set()


def load_conversation_record_strict(file_path):
    """A record off disk for the housekeeper, or ``None`` when it is unreadable.

    Unlike the turn's reader, this does not stand in an empty record for a
    damaged file: an empty record written over a damaged one would erase a
    conversation. ``None`` means "leave it alone".
    """
    try:
        record = json.loads(file_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(record, dict):
        return None
    if not isinstance(record.get("messages"), list) or not isinstance(record.get("summaries"), list):
        return None
    return record


def update_conversation_profile(file_path) -> bool:
    """Fold every row the profile has not read into the record, now.

    Called as each turn ends, and again by the idle pass for a record that has
    been left alone -- including one from before the profile existed, which is
    read from its whole transcript because a marker cannot speak for a section
    that is not there. A missing section is written even when the model finds
    nothing to record: its absence is what the next visit would read as a
    stranger's.

    The model is asked outside ``CONVERSATION_LOCK`` so a turn appending to the
    conversation is not made to wait behind it, and the record is reloaded
    before writing. The marker advances only to the newest row the model
    actually saw, so a turn that landed mid-call is read next time.

    Returns whether the record was written. A profile already up to date is
    not rewritten.
    """
    with CONVERSATION_PROFILE_LOCK:
        with CONVERSATION_LOCK:
            record = load_conversation_record_strict(file_path)
            if record is None:
                return False
            all_messages = [item for item in record.get("messages") or [] if isinstance(item, dict)]
            profile_missing = "user_details" not in record
            profile_messages = (
                all_messages
                if profile_missing
                else unprofiled_messages(all_messages, record.get("user_details_covered_through"))
            )
            # Assistant beats alone teach nothing about the person, but they
            # are context for the next user message, so they stay unread rather
            # than being marked covered.
            has_user_messages = any(str(item.get("role") or "") == "user" for item in profile_messages)
            has_work = bool(profile_messages) and has_user_messages
            write_empty_profile = profile_missing and not has_work
            if not has_work and not write_empty_profile:
                return False
            current_details = normalize_user_details(record.get("user_details"))
            covered_through = newest_message_timestamp(profile_messages)

        updated_details = (
            update_conversation_user_details(profile_messages, current_details) if has_work else {}
        )

        with CONVERSATION_LOCK:
            record = load_conversation_record_strict(file_path)
            if record is None:
                return False
            updated = dict(record)
            updated["user_details"] = updated_details
            if covered_through:
                updated["user_details_covered_through"] = covered_through
            write_conversation_record_file(file_path, updated)
        return True


def profile_conversation_worker(file_path):
    """Keep profiling one conversation until the record holds nothing new.

    Turns can land while the model is answering. Looping rather than starting a
    second worker is what makes "saved as the turn ends" true for the last turn
    as well as the first: the row that arrived mid-call is simply the next
    pass's work. The loop follows the marker rather than simply repeating, so a
    row whose timestamp cannot be read -- which every pass counts as unread --
    costs one retry, not an endless series of model calls.
    """
    last_marker = None
    try:
        while update_conversation_profile(file_path):
            with CONVERSATION_LOCK:
                record = load_conversation_record_strict(file_path)
            marker = str((record or {}).get("user_details_covered_through") or "")
            if marker == last_marker:
                break
            last_marker = marker
    except Exception:
        # Housekeeping will try again. A failed profile must never surface as a
        # turn's failure, because the reply has already been given.
        app.logger.warning(f"Conversation profile update failed for {file_path}", exc_info=True)
    finally:
        with CONVERSATION_PROFILE_WORKERS_LOCK:
            CONVERSATION_PROFILE_WORKERS.discard(file_path)


def start_conversation_profile_update(file_path) -> None:
    """Profile a conversation in the background, without delaying the turn.

    A worker already running for this file will see the new rows on its next
    pass, so a second thread would only wait for the same lock.
    """
    with CONVERSATION_PROFILE_WORKERS_LOCK:
        if file_path in CONVERSATION_PROFILE_WORKERS:
            return
        CONVERSATION_PROFILE_WORKERS.add(file_path)
    try:
        threading.Thread(
            target=profile_conversation_worker,
            args=(file_path,),
            name="conversation-profile",
            daemon=True,
        ).start()
    except Exception:
        with CONVERSATION_PROFILE_WORKERS_LOCK:
            CONVERSATION_PROFILE_WORKERS.discard(file_path)
        raise


def schedule_conversation_profile_update(conversation_key, character_profile) -> None:
    """Save the turn's profile, and never let that cost the turn.

    The reply has already been given by the time this runs, so nothing here --
    a legacy key the path builder refuses, a folder it cannot create, a thread
    the process will not start -- may raise into the caller.
    """
    try:
        file_path = conversation_record_path(
            conversation_key, character_profile=character_profile
        )
        start_conversation_profile_update(file_path)
    except Exception:
        app.logger.warning("Conversation profile update could not be started", exc_info=True)


def wait_for_conversation_profile_workers(timeout=5.0) -> None:
    """Wait for in-flight profile saves, up to ``timeout`` seconds.

    A turn never waits on this. It exists for an orderly shutdown and for tests
    that remove the folder a worker is still writing to.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with CONVERSATION_PROFILE_WORKERS_LOCK:
            if not CONVERSATION_PROFILE_WORKERS:
                return
        time.sleep(0.02)


def conversation_character_profile(file_path):
    """The character a record belongs to, for the summary's own framing.

    Root records belong to no character, and a folder without a readable
    outline still holds a conversation worth summarizing, so both answer
    ``None`` rather than refusing the work.
    """
    if file_path.parent == MEMORY_DIR:
        return None
    try:
        outline = json.loads((file_path.parent / CHARACTER_OUTLINE_FILENAME).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(outline, dict) or not outline.get("name") or not outline.get("locale"):
        return None
    return dict(outline)


def housekeep_conversation_record_file(file_path, now):
    """Trim and summarize one idle conversation, if it needs any.

    The profile belongs to the turn, not to this pass: it is written as each
    turn ends. It is brought up to date here as well, for a record that has
    been left alone since before the profile existed. That happens first, and
    a profile failure stops the pass before anything is dropped -- the rows it
    would have read are the only copy of what the person said.

    The summary is a model call outside ``CONVERSATION_LOCK`` so a turn
    appending to the same conversation is not made to wait behind it. The
    record is reloaded after the call and the first ``consumed_count`` rows are
    dropped: the transcript is append-only, so a turn that landed while the
    call ran sits after the consumed prefix and survives.
    """
    update_conversation_profile(file_path)

    with CONVERSATION_LOCK:
        record = load_conversation_record_strict(file_path)
        if record is None:
            return
        plan = plan_conversation_compaction(
            record,
            now=now,
            detailed_days=DETAILED_MEMORY_DAYS,
            summary_days=SUMMARY_MEMORY_DAYS,
            max_messages=CONVERSATION_MAX_MESSAGES,
        )
        stored_summaries = [item for item in record.get("summaries") or [] if isinstance(item, dict)]
        if (
            plan.consumed_count == 0
            and not plan.messages_to_summarize
            and plan.retained_summaries == stored_summaries
        ):
            return

    summary_text = ""
    if plan.messages_to_summarize:
        summary_text = summarize_conversation_messages(
            plan.messages_to_summarize,
            character_profile=conversation_character_profile(file_path),
        )

    with CONVERSATION_LOCK:
        record = load_conversation_record_strict(file_path)
        if record is None:
            return
        messages = list(record.get("messages") or [])
        if len(messages) < plan.consumed_count:
            # The file vanished and was rewritten by a turn while the model
            # was being asked. Those are not the rows this plan read.
            return
        summaries = list(plan.retained_summaries)
        if summary_text and plan.messages_to_summarize:
            summaries.append(
                {
                    "start_timestamp": str(plan.messages_to_summarize[0].get("timestamp") or ""),
                    "end_timestamp": str(plan.messages_to_summarize[-1].get("timestamp") or ""),
                    "summary": summary_text,
                }
            )
        summaries.sort(key=lambda item: str(item.get("end_timestamp") or ""))
        # Start from the record as it stands, not a record-shaped dict: the
        # delivery style, the profile and anything else beside the two lists is
        # part of the conversation, and a rewrite must not be how it is lost.
        updated = dict(record)
        updated["messages"] = messages[plan.consumed_count:]
        updated["summaries"] = summaries[-36:]
        write_conversation_record_file(file_path, updated)


def run_conversation_housekeeping(now=None):
    """One sweep over every idle conversation that has changed since the last.

    Files are examined once per mtime, so a housekeeping loop that runs every
    thirty seconds still reads an untouched conversation only once. Cooldown
    entries are what a failed pass leaves behind.

    Trimming and summarizing wait for the conversation to go quiet. The
    profile does not, and is normally saved by the turn; this pass catches a
    record that was left alone since before the profile existed.
    """
    now = now if now is not None else datetime.now().astimezone()
    with ACTIVE_CHAT_REQUESTS_LOCK:
        if ACTIVE_CHAT_REQUESTS > 0:
            # The summarizer shares the resident model with live turns; an idle
            # conversation's housekeeping can wait for the room to be quiet.
            return

    seen = set()
    for file_path in iter_conversation_record_files():
        if not names_a_conversation(file_path.name):
            # A legacy session-keyed record is never read by anything now, and
            # summarizing it would cost a model call to preserve words nobody
            # looks at. The character outline never reaches here (the iterator
            # skips it).
            continue
        seen.add(file_path)
        try:
            modified_at = file_path.stat().st_mtime
        except OSError:
            continue
        with CONVERSATION_HOUSEKEEPING_STATE_LOCK:
            if CONVERSATION_HOUSEKEEPING_CHECKED_MTIME.get(file_path) == modified_at:
                continue
            if CONVERSATION_HOUSEKEEPING_RETRY_AT.get(file_path, 0.0) > now.timestamp():
                continue
        if now.timestamp() - modified_at < CONVERSATION_IDLE_SECONDS:
            continue
        try:
            housekeep_conversation_record_file(file_path, now)
        except Exception:
            app.logger.warning(f"Conversation housekeeping failed for {file_path}", exc_info=True)
            with CONVERSATION_HOUSEKEEPING_STATE_LOCK:
                CONVERSATION_HOUSEKEEPING_RETRY_AT[file_path] = (
                    now.timestamp() + CONVERSATION_HOUSEKEEPING_COOLDOWN_SECONDS
                )
            continue
        try:
            examined_at = file_path.stat().st_mtime
        except OSError:
            examined_at = modified_at
        with CONVERSATION_HOUSEKEEPING_STATE_LOCK:
            CONVERSATION_HOUSEKEEPING_CHECKED_MTIME[file_path] = examined_at
            CONVERSATION_HOUSEKEEPING_RETRY_AT.pop(file_path, None)

    with CONVERSATION_HOUSEKEEPING_STATE_LOCK:
        for known in list(CONVERSATION_HOUSEKEEPING_CHECKED_MTIME):
            if known not in seen:
                CONVERSATION_HOUSEKEEPING_CHECKED_MTIME.pop(known, None)
        for known in list(CONVERSATION_HOUSEKEEPING_RETRY_AT):
            if known not in seen:
                CONVERSATION_HOUSEKEEPING_RETRY_AT.pop(known, None)


def conversation_housekeeping_loop():
    while True:
        try:
            run_conversation_housekeeping()
        except Exception:
            app.logger.exception("Conversation housekeeping sweep failed")
        time.sleep(CONVERSATION_HOUSEKEEPING_POLL_SECONDS)


def start_conversation_housekeeper():
    threading.Thread(
        target=conversation_housekeeping_loop,
        name="conversation-housekeeper",
        daemon=True,
    ).start()
```

## The one helper this slice must also add

`iter_conversation_record_files` (`agent.py:3388`) is the single definition of "every stored
record": root `*.json` first, then one level of character folders, skipping
`character-outline.json`. Three callers use it in the source (retention, clearing, this sweep) and
the docstring's point is that three different globs is how a file survives one rule and is swept by
another. Add it to `store.rs` as a method:

```rust
pub fn record_files(&self) -> std::io::Result<Vec<PathBuf>>   // root *.json, then memory_dir/*/*.json
                                                             // minus the character outline name
```

`CHARACTER_OUTLINE_FILENAME` is `"character-outline.json"`. Do not port the other two callers
(retention, clearing) — they are not Stage 4.

## Decisions already taken — do not re-derive

1. **The model calls are prompt-builders plus parsers, with the transport injected.** This crate's
   layer position is `config, json, text, store, models, prompts` — **not `shed-llm`** — so it must
   not open an HTTP connection. Declare one port in this crate:
   `pub trait BackgroundModel { fn complete_json(&self, purpose: &str, messages: &[ChatMessage]) -> Result<serde_json::Value, ModelError>; }`
   with `ChatMessage { role: String, content: String }` and
   `ModelError { transport(String), not_json, no_choice }` as its failure cases. `purpose` is the
   source's `purpose=` string (`"conversation summary"`, `"conversation user profile"`) so the
   caller can log it. The real implementation (retry ladder, `resolve_background_model`,
   `LM_ENGINE_HOST`) is the caller's, in the slice that wires the server.
2. **The provider answers with the parsed JSON body**, not a status code: the source's
   `status_code >= 400` and `_safe_body_snippet` belong to the transport. What stays here is
   `parse_json_response_strict`'s job: the `choices[0].message.content` string, then that string
   parsed as JSON. If `choices` is empty, the source uses `"{}"` — keep that.
3. **`summarize_conversation_messages` keeps its fallback**: an unreadable answer returns
   `" ; ".join(last two transcript lines)[:240]`; an empty transcript returns `""` and asks nothing.
   The transcript is built from `"{role}: {content}"` lines with empty contents skipped, then
   **truncated to the last 12,000 characters** (characters, not bytes — `chars().rev().take(12000)`).
4. **`update_conversation_user_details` raises, it never falls back**: unreadable JSON or a
   missing `user_details` object is `Err`, because its caller drops the rows it read — a silent
   success there would lose what the person said. An empty transcript returns
   `current_details` unchanged. The current profile is rendered with **sorted keys**
   (`ensure_ascii=False, sort_keys=True`) — `UserDetails` must render sorted for this prompt only,
   not for the file.
5. **The two `other`-specific rules** live in the prompt text and must be pasted unchanged: labels
   short and lower-case, `other` allowed 512 characters. The system text is part of the port.
6. **State is owned, not global.** No module-level `HashMap`s and no locks with static lifetime:
   `pub struct HousekeepingState { checked: HashMap<PathBuf, SystemTime>, retry_at: HashMap<PathBuf, SystemTime> }`
   passed as `&mut`, plus `pub struct ProfileWorkers { active: Mutex<HashSet<PathBuf>> }` with
   `start(&self, path)`, `wait(&self, timeout: Duration)`, and a `Drop` that waits. The Python's
   three module locks are what this replaces.
7. **The strict loader is separate from 4d's forgiving one.**
   `pub fn load_conversation_record_strict(path: &Path) -> Option<ConversationRecord>`: any IO or
   JSON error is `None`, and so is a document whose `messages`/`summaries` are absent or not
   arrays. **`None` means "leave the file alone"** — an empty record written over a damaged one
   would erase a conversation. Put it in `housekeeping.rs`, beside its only callers.
8. **The profile update keeps the source's shape**: lock → read strict → decide work → unlock →
   model call → lock → reload → write. `profile_missing` is "the record has no `user_details` key
   at all" (`Option::is_none()`); `write_empty_profile` is `profile_missing && !has_work`, and it
   writes `{}` **plus** nothing else, because the key's absence is what the next visit would read
   as a stranger. `has_work` requires at least one `user` row. The marker is
   `newest_message_timestamp` (4b) of the rows *actually seen*, and only set when non-empty.
   Returns whether the file was written.
9. **The worker loop follows the marker.** Loop while the update returns `true`; re-read the
   marker; if it did not advance, stop (a row whose timestamp cannot be read is unread forever and
   must not loop). Any error is logged and swallowed — the reply has already been given, so a
   profile failure must never surface into a turn. Always remove the path from the active set,
   including on error.
10. **`schedule_…` never raises.** Path building can fail on a legacy key; a thread can fail to
    start. Both are logged and swallowed. Take the record path (or the key + slug and the root) and
    return `()`.
11. **One worker per conversation**, not a queue: if the path is already active, return. New rows
    are picked up by the running pass.
12. **The sweep takes everything as parameters**:
    `pub fn run_housekeeping(store: &ConversationRoot, settings: &HousekeepingSettings, state: &mut HousekeepingState, active_chat_requests: usize, now: SystemTime) -> Vec<(PathBuf, SweepFailure)>`
    or the equivalent — `active_chat_requests > 0` returns immediately (the summariser shares the
    resident model with live turns). The loop itself (`conversation_housekeeping_loop`,
    `start_conversation_housekeeper`) is the binary's or the server's job; port the poll interval as
    a field and expose `run_housekeeping` plus a `spawn` that takes `poll: Duration`. Do not start a
    thread at import time.
13. **Settings are a struct, not env reads**:
    `HousekeepingSettings { detailed_days: f64, summary_days: f64, max_messages: i64, max_summaries: usize, idle: Duration, cooldown: Duration, poll: Duration }`.
    The source's defaults for the tests: detailed 7 days, summary 0, max_messages 10,
    max_summaries 36, idle 300 s, cooldown 900 s, poll 30 s. `shed-config` fills it later.
    The `summaries[-36:]` hard-coded in the source becomes `max_summaries`.
14. **`conversation_character_profile`** answers `Option<CharacterName { name, locale }>`: `None` for
    a root record, `None` when the outline is missing/unreadable, and `None` unless **both** `name`
    and `locale` are non-empty. It is used only to frame the summary prompt, so two fields are enough
    — do not invent a full outline type here.
15. **What "leaves the file alone" means**: a damaged record is never rewritten; a profile failure
    stops the pass **before** anything is dropped; a summary failure leaves the record untouched; a
    turn that landed during the model call survives (reload, compare `len(messages) < consumed_count`,
    then drop exactly `consumed_count`). Those four are the tests that matter most.

## Tests to port

`tests/test_conversation_housekeeping.py` — the remaining 25, in
`crates/shed-conversation/tests/housekeeping.rs`, with the model port faked (the Python's
`patch(...)` of the LM call becomes a `FakeModel` implementing `BackgroundModel`):

- `test_an_idle_conversation_past_the_cap_is_trimmed_and_summarized_once`
- `test_trimming_a_conversation_keeps_the_style_it_was_spoken_in` (the trimming half; 4d took the
  style-read half)
- `test_a_recently_written_conversation_is_left_alone`
- `test_a_turn_appended_during_the_summary_is_kept`
- `test_a_summarizer_failure_leaves_the_conversation_alone`
- `test_a_profile_failure_leaves_the_conversation_alone`
- `test_a_damaged_record_is_left_alone`
- `test_no_sweep_runs_while_a_chat_turn_is_in_flight`
- `test_the_character_outline_is_never_touched`
- `test_a_conversation_under_the_cap_still_learns_who_the_person_is`
- `test_a_missing_profile_is_backfilled_from_the_whole_conversation`
- `test_a_missing_profile_is_written_even_with_nothing_to_ask_about`
- `test_a_profile_only_reads_messages_it_has_not_seen`
- `test_trimming_keeps_the_profile_it_has_learned`
- `test_a_turn_that_lands_during_the_profile_is_read_next_time`
- `test_a_tail_of_reply_beats_alone_does_not_cost_a_profile_call`
- `test_the_profile_is_saved_as_the_turn_ends_not_after_the_idle_wait`
- `test_an_up_to_date_profile_is_not_rewritten`
- `test_a_turn_that_lands_during_the_save_is_still_read_immediately`
- `test_the_profile_scheduler_never_raises_into_a_turn`
- `test_a_legacy_key_is_not_profiled`
- `UserProfileRequestTests` (4): `test_the_model_sees_the_profile_and_the_messages_it_has_not_read`,
  `test_an_answer_without_a_profile_raises`, `test_a_prose_answer_raises`,
  `test_an_http_failure_raises`

Read those bodies: the fixtures pin exact JSON and exact marker strings, the "turn lands during the
call" tests inject a write at a precise moment through the fake model, and the profile tests count
model calls (0, 1, never 2).

## Gate (must be green before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # report the total you get; it was 433 before 4d
cmd /c "npm test 2>&1"                      # green, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Gate green** → commit `Stage 4 (part 2e): the idle housekeeper and the profile workers`, push,
   add one line to `TODO/RESUME-NOTES.md` under "Where we are" (4e committed, with the hash), and
   **stop**. Do not start 4f.
2. **Gate red after two honest fix attempts**, or the source and this brief disagree → **stop** and
   report the failing output plus the disagreement. Do not widen the slice.

## Standing rules for this repo

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- `cargo fmt --all` after writing; the workspace is `max_width = 100`.
- A dependency you add to a crate must also be listed in the root `[workspace.dependencies]` with
  the crate that consumes it, or `cargo clippy` fails at the workspace level.
- PowerShell: `;` not `&&`. Absolute paths everywhere — .NET APIs ignore the current directory.
- Tool calls: use the declared names, `files__read_file`, `files__write_file`, `files__edit_file`,
  `files__grep_files`, `files__list_dir`, `files__glob_files`, `terminal__run_command`. Call them
  through the tool call itself; never type `<function=...>` or `<parameter=...>` into your reply —
  that text is not executed and the turn ends with nothing done. One tool call per reply.
