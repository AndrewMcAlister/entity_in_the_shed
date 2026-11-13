# Slice 4b brief — timestamps, unread rows and the return gap

Hand this file to the session as its task. The source is pasted below and the decisions are
already made. **Do not read `agent.py`.** Do not read `conversation_history.py` beyond line 309
— lines 314+ are slice 4c (the user-details profile and the compaction plan).

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`, so a bare `TODO/...` resolves to `D:\TODO\...` and
fails: pass the full `D:\Entity_In_The_Shed\...` path to the files tool, and start every terminal
command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2 (this is slice 4b), §3.2/§3.3 (crate map, layers).
- Rules: `How_To_Code.md`. Workspace lints deny `dead_code`, `unused`,
  `unused_crate_dependencies`, `clippy::unwrap_used`, `clippy::expect_used` **including test
  targets**.
- The crate already exists from 4a: `crates/shed-conversation` with `record.rs` holding
  `ConversationMessage`, `ConversationSummary`, `ConversationRecord`.

## Mission

Add the timestamp grammar and the three rules built on it to `crates/shed-conversation`: which
stored rows a profile pass has not read, the newest stamp as it was written, and whether a
conversation has been quiet long enough to be a return. Pure functions, no I/O.

## The source, verbatim (`utils/conversation_history.py`, lines 219–309)

```python
def _parse_timestamp(value) -> datetime | None:
    if not isinstance(value, str):
        return None
    try:
        return datetime.fromisoformat(value)
    except ValueError:
        return None

def _is_at_or_after(value, cutoff) -> bool:
    """Whether a stamp falls inside a window, with unreadable stamps kept.

    A missing or malformed timestamp is not evidence of old age, so it counts
    as recent. Reading it as expired would let one bad row delete a memory.
    """
    parsed = _parse_timestamp(value)
    if parsed is None:
        return True
    if parsed.tzinfo is None and cutoff.tzinfo is not None:
        parsed = parsed.replace(tzinfo=cutoff.tzinfo)
    return parsed >= cutoff

def unprofiled_messages(messages, covered_through) -> list[ConversationMessage]:
    """The rows a profile pass has not read yet, oldest first.

    ``covered_through`` is the timestamp of the newest message an earlier pass
    gave the model, not the time of that pass. Comparing against it rather than
    counting rows is what lets the marker survive trimming: rows leave from the
    front, and the marker names a row that stays.

    A row whose timestamp cannot be read counts as unread. Skipping it would be
    the one direction that loses something the person said.
    """
    marker = _parse_timestamp(covered_through)
    unread = []
    for message in messages:
        if not isinstance(message, dict):
            continue
        if marker is None:
            unread.append(message)
            continue
        parsed = _parse_timestamp(message.get("timestamp"))
        if parsed is None:
            unread.append(message)
            continue
        if parsed.tzinfo is None and marker.tzinfo is not None:
            parsed = parsed.replace(tzinfo=marker.tzinfo)
        if parsed > marker:
            unread.append(message)
    return unread

def newest_message_timestamp(messages) -> str:
    """The newest readable timestamp in a block of rows, as it was written.

    The original spelling is returned rather than a re-serialized datetime, so
    the marker compares against later rows exactly as the file does.
    """
    newest_value = ""
    newest_parsed = None
    for message in messages:
        if not isinstance(message, dict):
            continue
        value = message.get("timestamp")
        parsed = _parse_timestamp(value)
        if parsed is None:
            continue
        if newest_parsed is None or parsed > newest_parsed:
            newest_parsed = parsed
            newest_value = str(value)
    return newest_value

def conversation_reopened(messages, *, now, quiet_seconds) -> bool:
    """Whether the stored conversation has been quiet long enough to be a return.

    A conversation is keyed by the browser and never ends, so "coming back" is
    the gap since its last message rather than a new file. A record with no
    readable stamp is not a return: there is no earlier visit to return from.
    ``quiet_seconds`` of zero disables the greeting rather than making every
    turn a reunion.
    """
    if quiet_seconds <= 0:
        return False
    newest = _parse_timestamp(newest_message_timestamp(messages))
    if newest is None:
        return False
    if newest.tzinfo is None and now.tzinfo is not None:
        newest = newest.replace(tzinfo=now.tzinfo)
    return (now - newest).total_seconds() >= quiet_seconds
```

## Decisions already taken — do not re-derive

1. **`datetime.fromisoformat` has no direct chrono equivalent.** Parse with a ladder, first
   match wins, and anything else is `None`:
   - `DateTime::parse_from_rfc3339` → aware (handles `Z` and `±HH:MM`)
   - `NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")` and
     `"%Y-%m-%d %H:%M:%S%.f"` → naive
   - `NaiveDate::parse_from_str(value, "%Y-%m-%d")` → midnight, naive (Python does the same)
   - **Time-only strings are not supported.** Python's 3.11 accepts some of them; every stored
     stamp in this app is a full date-plus-time. Say so in the module doc.
2. **One type for both kinds.** `pub enum Timestamp { Naive(NaiveDateTime), Aware(DateTime<FixedOffset>) }`.
3. **The mixing rule is `replace`, not conversion.** Wherever the Python does
   `parsed.replace(tzinfo=cutoff.tzinfo)` it *attaches* the other side's offset to the naive
   wall time — it does not convert the instant. Implement exactly that: a naive stamp compared
   against an aware one takes the aware side's offset and is then compared as an instant.
   A helper on `Timestamp` (`fn at_least(self, other: Timestamp) -> bool` or similar) is where
   this lives; the three callers must not each re-implement it.
4. **Unreadable is recent.** `is_at_or_after` returns `true` for a missing or unparseable stamp,
   and `unprofiled_messages` counts such a row as unread. This is the rule that stops one bad
   row deleting a memory — pin it with a test each way.
5. **`newest_message_timestamp` returns the string as written**, not a re-serialised datetime:
   `Option<&str>`/`Option<String>`, never a formatted `Timestamp`. The marker then compares
   against later rows exactly as the file spells them.
6. **Typed input.** The functions take `&[ConversationMessage]`, so Python's
   `if not isinstance(message, dict): continue` cannot occur and disappears. Note that in the
   module doc and add one `docs/PARITY.md` row: the Python tolerates a non-object row because
   it reads raw JSON; this crate types the row at load (slice 4d), so a malformed row fails
   there rather than being skipped here.
7. **Module split.** `src/timestamp.rs` (parse + comparison), `src/freshness.rs` (the three list
   functions). Re-export from `lib.rs`. `chrono` is added to `[workspace.dependencies]` by this slice (`default-features = false`, `std` only): it is the workspace's first date/time consumer, so it earns its place here rather than arriving ahead of one (principle 16).
8. **The 8 tests to port** (`tests/test_conversation_history_utils.py`):
   `no_marker_means_everything_is_unread`, `only_rows_after_the_marker_are_unread`,
   `a_row_without_a_timestamp_counts_as_unread`, `the_newest_timestamp_is_read_from_the_tail`,
   `a_gap_at_least_as_long_as_the_threshold_is_a_return`, `a_recent_message_is_not_a_return`,
   `zero_disables_the_greeting`, `a_record_with_no_readable_stamp_is_not_a_return`.
   Add two of your own for the mixing rule: a naive row compared with an aware marker
   (attached, not converted) and the reverse. Read the Python file only if a name is ambiguous.

## Standing rules for this repo

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- **PowerShell:** `;` not `&&`. Do not use `Get-Content f | Measure-Object -Line` to count lines.
  .NET APIs ignore PowerShell's current directory: pass absolute paths.
- `cargo fmt --all` after writing; the workspace is `max_width = 100`.

## Gate (must be green before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # 392 before this slice; 402 after
cmd /c "npm test 2>&1"                      # 250, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Gate green** → commit `Stage 4 (part 2b): shed-conversation timestamps, unread rows and the
   return gap`, push, add one line to `TODO/RESUME-NOTES.md` under "Where we are", and **stop**.
   Do not start 4c.
2. **Gate red after two honest fix attempts**, or the source and this brief disagree → **stop**
   and report the failing output plus the disagreement. Do not widen the slice.
