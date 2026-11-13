# Slice 4c brief — profile facts and the compaction plan

Hand this file to the session as its task. The source is pasted below and the decisions are
already made. **Do not read `agent.py`.** Do not read `conversation_history.py` beyond line 445
— it ends there.

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`, so a bare `TODO/...` resolves to `D:\TODO\...` and
fails: pass the full `D:\Entity_In_The_Shed\...` path to the files tool, and start every terminal
command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2, where this slice is already written down at **line 374**:
  `| **4c** | User-details profile + compaction plan | :314-445 |`. You do not need to search for
  it, and if a `findstr` for it comes back empty the search was wrong, not the plan. §3.2/§3.3
  hold the crate map and layers.
- **Do not spend a round confirming the slice exists.** There is no other copy of this task to
  find: the brief in your hands is it. Read it, open the two named files, write the two modules.
- Rules: `How_To_Code.md`. Workspace lints deny `dead_code`, `unused`,
  `unused_crate_dependencies`, `clippy::unwrap_used`, `clippy::expect_used` **including test
  targets**.
- The crate exists with `record.rs`, `timestamp.rs`, `freshness.rs` (4a, 4b). `window.rs` already
  has `message_window_start_index(messages: usize, max_messages: i64) -> usize`, and
  `timestamp.rs` already has `is_at_or_after(value: Option<&str>, cutoff: Timestamp) -> bool`.

## Mission

Two pure pieces, no I/O:

1. **`normalize_user_details`** — turn whatever the model returned into a flat, prompt-sized
   profile: one short phrase per topic, lists folded into one phrase, `other` kept aside with a
   paragraph's room.
2. **`plan_conversation_compaction`** — decide what a housekeeping pass trims and what it
   summarizes, without writing anything.

## The source, verbatim (`utils/conversation_history.py`, lines 312–445)

```python
# How much of a person's profile the prompt can carry: one short line per topic.
# The limits are the model's enthusiasm getting ahead of the prompt's budget.
USER_DETAILS_MAX_FACTS = 24
USER_DETAILS_MAX_VALUE_CHARS = 200
# ``other`` is the reserved catch-all for context with no natural topic of its
# own, so it gets a paragraph's room rather than a phrase's and a slot that the
# topic cap cannot take away.
USER_DETAILS_OTHER_LABEL = "other"
USER_DETAILS_OTHER_MAX_CHARS = 512


def _detail_value_text(value) -> str:
    """One profile value as a short phrase, or ``""`` when it is not one."""
    if isinstance(value, str):
        return " ".join(value.split())
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return str(value)
    if isinstance(value, (list, tuple)):
        parts = [_detail_value_text(item) for item in value]
        return ", ".join(part for part in parts if part)
    return ""


def _detail_label(key) -> str:
    """A topic key in the stored form: lower case, spaces become underscores."""
    return str(key or "").strip().lower().replace(" ", "_")


def normalize_user_details(value) -> dict[str, str]:
    """A flat, prompt-sized profile from whatever the model returned.

    The shape is deliberately simple: one short line per topic, keyed by the
    topic itself -- ``name``, ``family``, ``likes``. ``family`` is just
    another topic, and a flat map is what keeps the rendered profile to a
    handful of lines. Lists are folded into the same phrase form so a model
    answering ``["cats", "jazz"]`` and one answering ``"cats, jazz"`` store
    the same thing.

    ``other`` is reserved: a catch-all for context that does not fit a short
    label, kept apart from the topic count so the cap cannot drop it, and
    allowed a paragraph instead of a phrase.
    """
    if not isinstance(value, dict):
        return {}
    other = ""
    for key, fact in value.items():
        if _detail_label(key) == USER_DETAILS_OTHER_LABEL:
            other = _detail_value_text(fact)[:USER_DETAILS_OTHER_MAX_CHARS]
            break
    details = {}
    for key, fact in value.items():
        label = _detail_label(key)
        text = _detail_value_text(fact)
        if not label or not text or label == USER_DETAILS_OTHER_LABEL:
            continue
        details[label] = text[:USER_DETAILS_MAX_VALUE_CHARS]
        if len(details) >= USER_DETAILS_MAX_FACTS:
            break
    if other:
        details[USER_DETAILS_OTHER_LABEL] = other
    return details


def plan_conversation_compaction(
    record,
    *,
    now,
    detailed_days,
    summary_days,
    max_messages,
    max_summaries=36,
) -> ConversationCompactionPlan:
    """Decide the trimmed record without writing anything.

    Two cuts apply, and both take a prefix: by age (the detailed window) and by
    message count. The later cut wins, because material past either bound is
    past the promise.

    What is summarized is deliberately not the same as what is trimmed.
    Material pushed out by the message cap is always summarized, even with
    summary memory switched off -- the cap is what the user asked for, and
    dropping the overflow would lose conversation the cap exists to preserve.
    Age-trimmed material is summarized only inside the summary window; older
    than that it is dropped, as it always was.
    """
    messages = list(record.get("messages") or [])
    summaries = list(record.get("summaries") or [])

    if detailed_days > 0:
        detailed_cutoff = now - timedelta(days=detailed_days)
        age_cut = 0
        for message in messages:
            timestamp = message.get("timestamp") if isinstance(message, dict) else None
            if _is_at_or_after(timestamp, detailed_cutoff):
                break
            age_cut += 1
    else:
        age_cut = len(messages)

    message_cut = message_window_start_index(messages, max_messages)
    cut = max(age_cut, message_cut)

    summary_window = summary_days > 0
    summary_cutoff = now - timedelta(days=summary_days) if summary_window else None

    messages_to_summarize = []
    for index, message in enumerate(messages[:cut]):
        if not isinstance(message, dict):
            continue
        if index < message_cut or (
            summary_window and _is_at_or_after(message.get("timestamp"), summary_cutoff)
        ):
            messages_to_summarize.append(message)

    if summary_window:
        retained_summaries = [
            summary
            for summary in summaries
            if isinstance(summary, dict)
            and _is_at_or_after(summary.get("end_timestamp"), summary_cutoff)
        ]
    else:
        # Summaries are not age-pruned when summary memory is off: the
        # message overflow still writes one, and it must survive the write
        # that created it.
        retained_summaries = [summary for summary in summaries if isinstance(summary, dict)]

    retained_summaries.sort(key=lambda item: str(item.get("end_timestamp") or ""))
    return ConversationCompactionPlan(
        messages_to_summarize=messages_to_summarize,
        retained_messages=messages[cut:],
        retained_summaries=retained_summaries[-int(max_summaries):],
        consumed_count=cut,
    )
```

## Decisions already taken — do not re-derive

1. **Insertion order wins over `BTreeMap`.** This is the decision `SLICE-4a-BRIEF` deferred to
   4c. Python's `dict` keeps the order the model wrote the topics in, and that order is what the
   file stores and what the rendered profile shows; `ConversationRecord::user_details` is
   currently `Option<BTreeMap<String, String>>`, which would silently sort them. Replace it with
   a new ordered type — `pub struct UserDetails(Vec<(String, String)>)` — whose `Serialize`
   writes a JSON object in that order and whose `Deserialize` keeps the document's order.
   No new dependency (`indexmap` is not worth it for 24 pairs). Duplicate keys: Python's
   `json.loads` keeps the **last** value at the **first** key's position — do the same, and write
   a test for it. Update `record.rs`, its doc, and the 4a round-trip test if it names the type.
2. **`normalize_user_details` takes `&serde_json::Value`.** The source's `isinstance` ladder is
   exactly "whatever the model returned", and a JSON value is the only shape that can carry an
   int, a float, a list and an object in one type. Add `serde_json` to the crate's normal
   dependencies (it is currently dev-only) — `unused_crate_dependencies` is per target, so keep
   the dev entry or move it, whichever keeps both targets clean.
3. **`_detail_value_text` maps as the source does**, and the odd cases matter:
   - string → whitespace collapsed (`" ".join(value.split())`: any run of whitespace becomes one
     space, and leading/trailing whitespace goes)
   - number → its **JSON spelling** (`Number::to_string()`), so `1.0` stays `"1.0"` where
     `f64::to_string` would say `"1"`
   - bool → `""`, **not** `"true"`: Python's `isinstance(True, int)` is true, so the source
     guards against it explicitly
   - array → each element rendered by the same function and joined with `", "`, empty parts
     dropped (`[None, "cats"]` → `"cats"`; `[]` → `""`)
   - object, null → `""`
4. **Truncation counts characters, not bytes.** Python slices strings by code point, so
   `[:200]` on a value of `"é"`x300 keeps 200 characters. Use `.chars().take(n)`. Pin it with one
   test that uses a multi-byte character; getting this wrong shows up only for non-ASCII names.
5. **`_detail_label` in Rust.** Keys arrive as `String` from JSON, so `str(key or "")` cannot
   fail; keep `.strip().lower().replace(' ', '_')` exactly — lowercase is `to_lowercase()`, and
   Python lowercases Unicode the same way for the scripts this app sees. A non-string key cannot
   occur (JSON keys are strings), so the source's `None: "no key"` case is untestable here: note
   it in the module doc, add a `docs/PARITY.md` row only if you think it worth a line.
6. **`other` is computed first and appended last**, so the 24-topic cap cannot drop it and it
   keeps its own 512-character room. The `break` after the first `other` key is the source's:
   only the first one counts.
7. **`plan_conversation_compaction` takes typed inputs**: `record: &ConversationRecord`,
   `now: DateTime<Utc>`, `detailed_days: f64`, `summary_days: f64`, `max_messages: i64`,
   `max_summaries: usize = 36`. Python's `timedelta(days=…)` accepts a float, so convert with
   `chrono::Duration::milliseconds((days * 86_400_000.0) as i64)`; build the cutoff as
   `Timestamp::Aware((now - span).fixed_offset())` and compare with the existing
   `is_at_or_after`. The `isinstance(message, dict)` skips disappear (typed rows, already a
   PARITY row from 4b).
8. **`detailed_days <= 0` means "trim everything by age"** (`age_cut = messages.len()`), not
   "keep everything". This is the source's `else`, and `a_zero_detailed_window_trims_everything`
   pins it.
9. **The two cuts take a prefix; the later wins**: `cut = max(age_cut, message_cut)`.
   `message_cut` comes from the existing `message_window_start_index(messages.len(), max_messages)`.
10. **What is summarized is not what is trimmed.** A row in `messages[..cut]` is summarized when
    `index < message_cut` (the message cap always summarizes its overflow, even with summary
    memory off) **or** when the summary window is on and its stamp is inside it. Rows older than
    the summary window and outside the message cap are dropped without a summary.
11. **Retained summaries**: pruned by `end_timestamp` only when the summary window is on;
    otherwise all are kept (the overflow summary must survive the write that created it).
    Sort by `end_timestamp` **as a string** (lexicographic — ISO-8601 sorts correctly and that is
    what the source does), then keep the **last** `max_summaries`. Python's `[-0:]` keeps
    everything, so `max_summaries == 0` must keep all summaries too; pin that quirk with a test.
12. **`Retained` = `messages[cut..]` cloned, `consumed_count = cut`.** The plan struct already
    exists in `record.rs` as `ConversationCompactionPlan`; keep its four fields and its doc.
13. **Module split**: `src/profile.rs` (constants, `detail_value_text`, `detail_label`,
    `UserDetails`, `normalize_user_details`) and `src/compaction.rs`
    (`plan_conversation_compaction`). Re-export from `lib.rs`.
14. **The 20 tests to port** (`tests/test_conversation_history_utils.py`), `ProfileFactsTests`:
    `labelled_facts_are_kept_as_short_phrases`, `a_list_of_preferences_becomes_one_phrase`,
    `an_age_given_as_a_number_still_reads`, `whitespace_is_collapsed_to_one_line`,
    `empty_labels_and_values_are_dropped`, `something_that_is_not_an_object_is_no_profile`,
    `the_profile_is_bounded`, `other_is_a_catch_all_with_its_own_room`,
    `other_is_cut_at_its_own_limit`, `a_labelled_topic_does_not_get_others_room`,
    `other_is_kept_even_when_the_topic_count_is_full`.
    `PlanCompactionTests`: `a_conversation_under_the_cap_is_left_alone`,
    `rows_past_the_message_window_are_summarized_even_with_summary_memory_off`,
    `material_older_than_the_summary_window_is_dropped_without_a_summary`,
    `a_missing_timestamp_counts_as_recent`, `a_zero_detailed_window_trims_everything`,
    `summaries_are_kept_when_summary_memory_is_disabled`,
    `summaries_ending_outside_the_window_are_pruned`,
    `the_newest_summaries_are_kept_up_to_the_limit`,
    `the_age_cut_wins_when_it_reaches_further_than_the_message_cut`.
    Read those test bodies: the fixtures pin exact field values, and the compaction ones choose
    their cutoffs so the age cut and the message cut disagree on purpose.

## Standing rules for this repo

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- `cargo fmt --all` after writing; the workspace is `max_width = 100`.
- A dependency you add to a crate must also be listed in the root `[workspace.dependencies]`
  with the crate that consumes it, or `cargo clippy` fails at the workspace level.

## Gate (must be green before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # 409 before this slice; 429 after
cmd /c "npm test 2>&1"                      # 250, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Gate green** → commit `Stage 4 (part 2c): shed-conversation profile facts and the
   compaction plan`, push, add one line to `TODO/RESUME-NOTES.md` under "Where we are", and
   **stop**. Do not start 4d.
2. **Gate red after two honest fix attempts**, or the source and this brief disagree → **stop**
   and report the failing output plus the disagreement. Do not widen the slice.
