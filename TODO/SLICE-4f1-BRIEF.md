# Slice 4f-1 brief — the reply layer: text, scanner, splitter, publisher, parsers

Hand this file to the session as its task. It is an implementation brief: **the source it needs is
pasted below**, byte-for-byte, and was checked against the reference app before this file was
written. Do not read `agent.py` in the session; the ranges you need are already here.

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`: pass the full `D:\Entity_In_The_Shed\...` path to the
files tool, and start every terminal command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2 line 377 (the 4f row) and §12 (definition of done).
- Scoping brief for the whole of 4f, and the reason it splits: `TODO/SLICE-4f-BRIEF.md`.
- Branch `main`. HEAD when this brief was written: `dc74524 Stage 4 (part 2e): the idle
  housekeeper and the profile workers`. Slices 4a–4e are committed and the crate totals were
  124 (`shed-conversation`) and 484 (workspace) at that commit; your first gate run will tell you
  where you actually are.

## Mission

Port the **pure reply layer** — the code that decides what the model's text *is* — into a new crate
`shed-chat`, with the reference suite's tests ported alongside it. This is the first third of 4f;
4f-2 adds `run_agent`, recovery and the three runtime guards, and 4f-3 adds `POST /api/chat` and
Stage 4's exit gate. Nothing in this sub-slice may start a thread, open a file or bind a socket:
the layer is fed by callers, and that is what makes 4f-2 and 4f-3 testable separately.

Two Python files are the whole of it:

| source | lines | what it is |
|---|---|---|
| `utils/reply.py` in full | 389 | reply-quality judgements and the prose tool-call envelope: 16 functions, no I/O |
| `agent.py:5504-6202` | 699 | reply text normalisation, the streaming JSON scanner, the sentence splitter, the publisher, and the reply-document parsers: 13 symbols |

## What to create

`shed-chat` is created here (plan D4: crates are created on demand). Nothing else is created; the
crate is the composition point for 4f-2 and 4f-3, so it is worth laying out for them now.

```
crates/shed-chat/
  Cargo.toml
  src/lib.rs            module list and the crate's public surface; doc comment naming 4f-1/2/3
  src/reply_text.rs     normalize_reply_text, markdown_to_ssml
  src/quality.rs        the 16 functions of utils/reply.py
  src/document.rs       _parse_structured_candidate, _parse_model_reply_document,
                        reply_without_prose_tool_calls, settle_prose_tool_calls,
                        parse_model_json_content
  src/stream_scanner.rs JsonReplyStreamScanner
  src/splitter.rs       SentenceSplitter
  src/streamed.rs       _streamed_prefix_length, split_reply_after_stream, strip_streamed_prefix
  src/publisher.rs      ReplyStreamPublisher (scanner + splitter + the prefix rules)
  tests/reply_text.rs tests/quality.rs tests/document.rs
  tests/stream_scanner.rs tests/splitter.rs tests/publisher.rs
  tests/fixtures/       the D8 golden files (see below)
```

Rules that the gate enforces, not style preferences:

- **`Cargo.toml` mirrors `crates/shed-conversation/Cargo.toml`**: `version.workspace`, `edition.workspace`,
  `publish = false`, `[lints] workspace = true`, a `description` that says what the crate is for, and
  a comment beside every dependency saying which module made it necessary.
- **Every dependency is in the root `[workspace.dependencies]`** with the consuming crate named in its
  comment. This sub-slice should need only `serde_json` (the reply documents) and `shed-text`
  (`py_str` for `str(value or "")`, `text_or_empty` for optional fields). If you reach for anything
  else, stop and say why in the commit message.
- **`crates/shed-gates/src/layers.rs` gains a `shed-chat` entry** at its layer: allowed edges are
  `config, json, text, models, llm, prompts, tools, mcp, conversation` (the plan's layer map). List
  *only* the edges this sub-slice actually uses — the honesty gate reads these lists, and an edge
  with no `use` behind it is a claim the code does not support. `shed-chat` may not depend on
  `shed-http` or `shed-server`: the composition root is above it, not below.
- **`unused_crate_dependencies` is denied per test binary.** A new crate with no callers fails the
  gate; 4f-1's own tests are the callers, so every public item is exercised from `tests/`.

## The port map

Python's duck typing becomes Rust's named types. Keep names, keep behaviour, keep the *order* of the
state machines; the tests are what prove the order. In the tables below, "1:1" means the same name,
same semantics, same error surface.

### `utils/reply.py` → `src/quality.rs`

| Python | Rust | notes |
|---|---|---|
| `looks_like_action_teaser_reply(reply_text)` | `pub fn looks_like_action_teaser_reply(reply_text: &str) -> bool` | an announcement ("I'll have a look") rather than a result; the boundary is a **word count**, not a phrase |
| `looks_like_mctrader_action_request(user_text)` | same shape | user text asking for a trade action |
| `looks_incomplete(reply_text, response_details=None)` | `pub fn looks_incomplete(reply_text: &str, response_details: Option<&Value>) -> bool` | `done_reason == "length"`, a trailing colon/ellipsis, or a long multi-line reply with no terminal punctuation; a complete sentence is not incomplete |
| `latest_user_message_text(messages)` | `pub fn latest_user_message_text(messages: &[Value]) -> String` | the last `role == "user"` message's text |
| `looks_like_task_stub_reply(user_text, reply_text)` | same shape | a promise with no content when the user asked for something |
| `merge_completed_reply(partial, completion)` | `pub fn merge_completed_reply(partial: &str, completion: &str) -> String` | joins the continuation onto the partial, removing the overlap it repeats |
| `extract_reply_segments(value)` | `pub fn extract_reply_segments(value: &Value) -> Vec<String>` | at most `REPLY_SEGMENT_MAX` (4) bubbles; a constant, not a magic number |
| `strip_tool_call_envelope(text)` | same | removes the prose `{tool_calls}` envelope, byte-identical when absent |
| `tool_calls_in_envelope(text)` | `pub fn tool_calls_in_envelope(text: &str) -> Vec<Value>` | |
| `_tool_calls_in_envelope_body(body)` | private | |
| `_normalize_tool_call(item)` | `fn normalize_tool_call(item: &Value) -> Option<Value>` | Python's `None` for "not a call" is `Option::None` |
| `looks_unusable_reply(reply_text)` | same | empty, whitespace, or nothing but punctuation |
| `looks_out_of_character_reply(reply_text)` | same | |
| `looks_like_recovery_meta_reply(reply_text)` | same | the model talking about the recovery attempt |
| `looks_like_generic_handoff_reply(user_text, reply_text)` | same | |
| `_is_duplicate_event(event_id, lock, recent_ids)` | `pub struct RecentEvents` + `pub fn is_duplicate(&mut self, event_id: &str) -> bool` | the source passes the module's lock and list in; the Rust type owns them. **Name the type** — 4f-2's guard uses it |

### `agent.py:5504-6202` → `src/{reply_text,document,stream_scanner,splitter,streamed,publisher}.rs`

| Python | Rust | notes |
|---|---|---|
| `normalize_reply_text(value)` | `reply_text::normalize_reply_text(value: &Value) -> String` | Python is `str(value or "")`: the **falsy test comes first** (`null`, `false`, `0`, `""`, `[]`, `{}` → `""`), *then* the string conversion. Use `shed_text::py_str` for the conversion itself, or `0` becomes `"0"` |
| `markdown_to_ssml(text)` | `reply_text::markdown_to_ssml(text: &str) -> String` | Windows TTS only. Keep the substitution order; the fixtures pin it |
| `JsonReplyStreamScanner` (class) | `stream_scanner::JsonReplyStreamScanner` | pulls the `reply` string out of a JSON document **while it is still arriving**. It recognises only the layout the prompt asks for; `parse_model_json_content` is the authority on everything else. Keep its methods and its state transitions; feed it one `&str` chunk at a time |
| `SentenceSplitter` (class) | `splitter::SentenceSplitter` | releases only text that is safe to speak alone; anything ambiguous waits for more input — a split inside `{tone}`, `[scene]`, `(aside)`, `*`, or an unbalanced bracket |
| `_streamed_prefix_length(reply, spoken)` | `streamed::streamed_prefix_length(reply: &str, spoken: &str) -> Option<usize>` | whitespace is not compared (the splitter strips each sentence); everything else is. `None` = the streamed text is not a prefix of the final reply |
| `split_reply_after_stream(reply, spoken)` | `streamed::split_reply_after_stream(reply: &str, spoken: &str) -> (String, bool)` | `(remaining, replaced)` |
| `strip_streamed_prefix(reply, spoken)` | `streamed::strip_streamed_prefix(reply: &str, spoken: &str) -> String` | |
| `ReplyStreamPublisher` (class) | `publisher::ReplyStreamPublisher` | sits between the model and the chat job: `feed` decodes as it arrives and publishes each finished sentence, `finish` flushes what is safe. Where Python takes a `publish` callback, take `&mut dyn FnMut(&str)`; do **not** spawn a thread or hold a lock across a feed |
| `_parse_structured_candidate(candidate)` | `document::parse_structured_candidate(candidate: &str) -> Option<Value>` | JSON first, then the Python-literal-dict fallback (single quotes, trailing comma, `None`). The fallback is not optional: models answer with it often enough to matter |
| `_parse_model_reply_document(content)` | `document::parse_model_reply_document(content: &str) -> ReplyDocument` | always answers a document: `{ "reply": "", "tool_calls": [] }` for empty input. A `ReplyDocument { reply: String, tool_calls: Vec<Value> }` is this sub-slice's typed form |
| `reply_without_prose_tool_calls(text)` | `document::reply_without_prose_tool_calls(text: &str) -> String` | renormalise **only** when an envelope was removed, so a reply that never had one is byte-identical |
| `settle_prose_tool_calls(parsed)` | `document::settle_prose_tool_calls(parsed: &mut ReplyDocument)` | moves any envelope out of `reply` and into `tool_calls` |
| `parse_model_json_content(content)` | `document::parse_model_json_content(content: &str) -> ReplyDocument` | the authority: raw model text → `reply` + `tool_calls`, including a reparse of a double-encoded string |

Where Python raises, the Rust returns `Option`/`Result` only if the source does something with the
failure; otherwise the source's empty value is the answer, because these functions are *judgements*,
not operations. Do not invent an error type for a function whose Python returns a bool.

## The D8 fixtures — captured, never booted

The golden files are the part a future session cannot regenerate from memory, so capture them now,
in this sub-slice, and commit them:

- **Where**: `crates/shed-chat/tests/fixtures/`, one JSON file per module (`reply_text.json`,
  `quality.json`, `document.json`, `stream_scanner.json`, `splitter.json`, `publisher.json`).
- **How**: a script under `tools/` (create `tools/capture_reply_fixtures.py` if `tools/` has no such
  script). For `utils/reply.py`, import the module (it imports only `json` and `re`). For the
  `agent.py` symbols, **do not import `agent.py`** — it is an 11,607-line application module. Read
  the source range 5504-6202 out of the file, `exec` it in a prepared namespace (`json`, `re`,
  `typing`, and the little else the pasted code touches — the script should fail loudly if a name is
  missing), and call the functions from there. That is still "the pure Python module in a
  subprocess", and it boots nothing.
- **Never** boot the reference app, bind 5005, or start a model.
- **Cases to capture, per module**: for the booleans and strings, a table of input → expected; for
  the scanner and publisher, the *chunk sequences* (whole, one character at a time, and two
  arbitrary but **seeded** splits) → the expected published text; for the splitter, each tricky
  token class; for the parsers, a document that is JSON, one that is a Python dict repr, one that is
  double-encoded, one that is empty, and one with a prose envelope. The fixtures are the oracle;
  the Rust tests read them, they do not restate them.
- The capture script is committed and re-runnable: the brief's author verified the pasted source is
  byte-identical, and the script is how a reader verifies it again.

## The tests

**Port every test: 136 of them**, plus one fixture-driven test per fixture file. The names are the
specification — the four modules and their counts:

| module | tests |
|---|---|
| `tests/test_reply_utils.py` | 78 |
| `tests/test_sentence_splitter.py` | 21 |
| `tests/test_reply_stream_scanner.py` | 20 |
| `tests/test_reply_stream_publisher.py` | 17 |

The full list follows the source; each test keeps its Python name transliterated to snake_case (the
names already are), and each keeps its assertions. Where a Python test reaches into a private helper
(`_streamed_prefix_length`, `_tool_calls_in_envelope_body`), the Rust test may too, from inside the
crate's test module — mark those `#[allow(clippy::...)]` only if the workspace denies what the test
must do, and say why in a comment.

Two of the test modules depend on `test_glm4.py` and `test_glm4_followup.py`: they are fixture
modules, not suites — read them for the reply documents they build and put those documents in
`document.json` rather than porting the modules.

## Done when

1. `cargo test -p shed-chat` reports **at least 136** passing and 0 failing, plus the fixture tests.
2. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` and `npm test` are all green, and the workspace total is the 484 from
   `dc74524` plus your tests (report the number you get; the plan's running total is a sum, not a
   target).
3. The crate starts nothing: no `std::thread`, no `std::fs`, no socket, no `tokio` runtime. The only
   dependencies are the ones listed above.
4. `crates/shed-gates/src/layers.rs` lists `shed-chat` with exactly the edges used.

## Order of work

1. Crate skeleton: `Cargo.toml`, `src/lib.rs`, the layer-map entry, `serde_json`/`shed-text` in the
   workspace list. Prove it with `cargo test -p shed-chat` (zero tests is a green run and a bad
   sign — add the first test with the first function).
2. `reply_text` (2 functions) and its tests. This is where the `py_str`/falsy subtlety is pinned.
3. `quality` (16 functions) with `test_reply_utils` ported as you go — it is 78 of the 136, so do it
   in the source's own order, one function and its tests at a time.
4. `document` (5 functions) plus the GLM4 fixture documents.
5. `stream_scanner` and its 20 tests, then `splitter` and its 21. Both are state machines; port the
   state transitions in the source's order and let the captured chunk sequences tell you when the
   order is wrong.
6. `streamed` (3 functions) and `publisher`, with the publisher's 17 tests last, because the
   publisher is where the other two meet.
7. Full gate, `cargo fmt --all`, commit `Stage 4 (part 2f-1): the reply layer`.

## Gate (before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # report the total you get
cmd /c "npm test 2>&1"                      # green, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Gate red after two honest fix attempts** → stop, report the output, do not widen the slice.
2. **The source in this file disagrees with a test** → the test wins only if it is in the pasted
   list; otherwise report the disagreement rather than "fixing" the port to match your reading.
3. **You find yourself wanting `shed-http` or `shed-server`** → stop. That is 4f-3, and the layer
   map forbids the edge.
4. **You find yourself wanting to look at `agent.py`** → the range you need is in this file. If
   something outside it is genuinely required, stop and say what and why; do not read the application
   module to fill a gap.

## Standing rules for this repo

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- `cargo fmt --all` after writing; the workspace is `max_width = 100`.
- A dependency you add to a crate must also be listed in the root `[workspace.dependencies]` with
  the crate that consumes it.
- PowerShell: `;` not `&&`. Absolute paths everywhere — .NET APIs ignore the current directory.
- Tool calls: use the declared names, `files__read_file`, `files__write_file`, `files__edit_file`,
  `files__grep_files`, `files__list_dir`, `files__glob_files`, `terminal__run_command`. Call them
  through the tool call itself; never type `<function=...>` or `<parameter=...>` into your reply —
  that text is not executed and the turn ends with nothing done. One tool call per reply.
- Commit message: `Stage 4 (part 2f-1): the reply layer`, then a body that names the crate, the
  module list, the test counts (136 + fixtures) and the gate totals.

## The source, verbatim

### `utils/reply.py` — the whole file, 389 lines

````python
"""Reply-quality and event-dedupe helpers extracted from agent.py.

agent.py re-imports these as aliases (thin facade) so existing call sites
and test ``@patch('agent.<func>')`` references keep working.

``is_duplicate_event`` reads agent.py module state (``RECENT_EVENT_IDS`` /
``RECENT_EVENT_LOCK``), so it is parameterized and wrapped in agent.py.
"""
import json
import re
from typing import Any

# How many separate bubbles one turn may produce. Small on purpose: the point is
# genuinely separate beats, not splitting every answer into fragments, which
# reads as a stutter rather than an entity talking.
REPLY_SEGMENT_MAX = 4


# A reply that announces work instead of reporting its result is short by
# nature: the announcement is the whole message. Real answers that happen to
# contain a phrase like "checking" are long. The limit keeps the patterns below
# from firing on those, which matters because the consumer at
# agent.py:resolve_followup_action_request re-issues the prior request when it
# sees a teaser -- a false positive there is a wrong turn, not a wasted call.
TEASER_MAX_WORDS = 40

# Grouped by frame rather than by verb so the list stays readable.
_TEASER_PATTERNS = (
    # Explicit retrieval verbs.
    r"\blet me\s+(?:fetch|get|check|pull|retrieve|look up)\b",
    r"\bi(?:'ll| will| shall)\s+(?:fetch|get|check|pull|retrieve|look up)\b",
    r"\bfetching\b",
    r"\bchecking\b",
    # Promise of a look or an investigation. This is the axis that was missing:
    # the reported failure was "Let me take a closer look.", which announced work
    # and never reported a result, and none of the patterns above match it
    # because "take" is not a retrieval verb.
    r"\blet me\s+(?:take|have)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\bi(?:'ll| will| shall)\s+(?:take|have)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\b(?:taking|having)\s+(?:a|another)\s+(?:closer\s+|quick\s+|good\s+|brief\s+)?look\b",
    r"\blet me\s+(?:look|see|find out|dig into|examine|investigate|review|go over|work through)\b",
    r"\bi(?:'ll| will| shall)\s+(?:look|see|find out|dig into|examine|investigate|review|go over|work through)\b",
    r"\b(?:looking|digging)\s+into\b",
    r"\bsee what i can (?:find|do|dig up)\b",
    r"\b(?:investigating|examining)\b",
)


def looks_like_action_teaser_reply(reply_text: object) -> bool:
    text = re.sub(r"\s+", " ", str(reply_text or "").strip().lower())
    if not text:
        return False
    if len(text.split()) > TEASER_MAX_WORDS:
        return False

    return any(re.search(pattern, text) for pattern in _TEASER_PATTERNS)


def looks_like_mctrader_action_request(user_text: object) -> bool:
    text = re.sub(r"\s+", " ", str(user_text or "").strip().lower())
    if not text:
        return False

    has_trader_context = any(token in text for token in ("mctrader", "mc trader", "matt trader", "trader", "broker", "watchlist", "portfolio"))
    has_action_intent = any(
        token in text
        for token in (
            "position",
            "positions",
            "account",
            "balance",
            "equity",
            "status",
            "running",
            "watchlist",
            "symbol",
            "symbols",
            "ticker",
            "analyse",
            "analyze",
            "analysis",
            "broker",
            "exchange",
            "start trader",
            "stop trader",
            "restart trader",
        )
    )
    return has_trader_context and has_action_intent


def looks_incomplete(reply_text: object, response_details: dict[str, Any] | None = None) -> bool:
    if not reply_text:
        return True

    response_meta = response_details.get("_meta") if isinstance(response_details, dict) else None
    if str((response_meta or {}).get("done_reason", "")).strip().lower() == "length":
        return True

    stripped = reply_text.strip()
    if stripped.endswith(":") or stripped.endswith("..."):
        return True

    if len(stripped) >= 60 and "\n" in stripped and not re.search(r"[.!?\"')\]\u201d]\s*$", stripped):
        return True

    lowered = stripped.lower()
    teaser_phrases = [
        "here's a short poem",
        "here is a short poem",
        "here's your short poem",
        "here is your short poem",
        "here's a concise poem",
        "here is a concise poem",
        "here's a poem",
        "here is a poem",
        "here is the poem",
    ]

    if any(phrase in lowered for phrase in teaser_phrases) and len(stripped.split()) <= 25:
        return True

    return False


def latest_user_message_text(messages: list[dict[str, Any]] | None) -> str:
    for message in reversed(messages or []):
        if message.get("role") == "user":
            return str(message.get("content", "")).strip()
    return ""


def looks_like_task_stub_reply(user_text: object, reply_text: object) -> bool:
    user_lower = str(user_text or "").strip().lower()
    reply = str(reply_text or "").strip()
    if not user_lower or not reply:
        return False

    generative_request_patterns = (
        r"\b(write|draft|compose|create|make|generate)\b",
        r"\b(poem|story|letter|email|speech|song|essay|script|paragraph|list)\b",
    )
    if not all(re.search(pattern, user_lower) for pattern in generative_request_patterns):
        return False

    reply_lower = reply.lower()
    if "\n" in reply:
        return False
    if len(reply.split()) > 18:
        return False

    stub_patterns = (
        r"^(i will|i'll|i shall)\b",
        r"^(here(?:'s| is)|certainly|sure|alright|okay|ok)\b",
        r"\b(write|draft|compose|create|make)\b.*\b(for you|now)\b",
    )
    if not any(re.search(pattern, reply_lower) for pattern in stub_patterns):
        return False

    content_markers = (
        '"',
        "'",
        ",",
        ";",
        ":",
    )
    return not any(marker in reply for marker in content_markers)


def merge_completed_reply(partial_reply: object, completion_reply: object) -> str:
    partial = str(partial_reply or "")
    completion = str(completion_reply or "")
    if not completion.strip():
        return partial.strip()

    partial_stripped = partial.strip()
    completion_stripped = completion.strip()
    if not partial_stripped:
        return completion_stripped
    if completion_stripped.startswith(partial_stripped):
        return completion_stripped

    max_overlap = min(len(partial_stripped), len(completion_stripped), 240)
    overlap = 0
    for size in range(max_overlap, 0, -1):
        if partial_stripped.endswith(completion_stripped[:size]):
            overlap = size
            break

    if overlap == 0 and re.match(r"^[A-Z\"']", completion_stripped):
        return completion_stripped

    suffix = completion_stripped[overlap:]
    if not suffix:
        return partial_stripped

    joiner = ""
    if partial_stripped[-1:].isalnum() and suffix[:1].isalnum():
        joiner = " "
    return f"{partial_stripped}{joiner}{suffix}".strip()


def extract_reply_segments(value: object) -> list[str]:
    """Split a model's ``replies`` field into clean, capped segments.

    Returns ``[]`` when there is nothing usable, which is the caller's signal to
    fall back to the single ``reply`` string. One turn may legitimately produce
    several distinct beats -- a preface and a result -- and they should reach the
    browser as separate bubbles rather than one growing one.

    Purely textual: it strips, drops non-strings and empties, and caps the count.
    It deliberately does *not* normalise, because ``normalize_reply_text`` lives
    in agent.py and importing it here would make this module depend on the app it
    was extracted from.
    """
    if isinstance(value, str):
        candidates: list[object] = [value]
    elif isinstance(value, (list, tuple)):
        candidates = list(value)
    else:
        return []

    segments: list[str] = []
    for item in candidates:
        if not isinstance(item, str):
            continue
        text = item.strip()
        if not text:
            continue
        segments.append(text)
        if len(segments) >= REPLY_SEGMENT_MAX:
            break
    return segments


# The prompt asks for a tool call as a JSON field, but a model sometimes writes the
# envelope as prose instead -- `{tool_calls}{"tool": ...}{/tool_calls}` -- borrowing the
# tone-tag syntax that the same prompt teaches a few lines below the schema. The tag has
# to literally name tool_calls for this to match, which is what keeps a real tone tag
# from ever being read as an envelope and stripped out of what the user hears.
TOOL_CALL_ENVELOPE_PATTERN = re.compile(
    r"\{\s*tool_calls\s*\}(?P<body>.*?)\{\s*/\s*tool_calls\s*\}",
    flags=re.IGNORECASE | re.DOTALL,
)

# An envelope that was opened and never closed leaves its tag behind. A tag naming
# tool_calls is markup wherever it appears, so it always goes; what it was carrying is
# left as written, because guessing where an unterminated envelope ends would sometimes
# delete a real answer.
TOOL_CALL_TAG_PATTERN = re.compile(r"\{\s*/?\s*tool_calls\s*\}", flags=re.IGNORECASE)


def strip_tool_call_envelope(text: object) -> str:
    """Remove any tool-call envelope a model wrote as prose rather than as JSON.

    Each envelope becomes a single space, so "now.{...}" reads "now. " rather than
    "now.". Callers normalise.
    """
    without_envelope = TOOL_CALL_ENVELOPE_PATTERN.sub(" ", str(text or ""))
    return TOOL_CALL_TAG_PATTERN.sub(" ", without_envelope)


def tool_calls_in_envelope(text: object) -> list[dict[str, Any]]:
    """The tool calls carried in a prose envelope, or ``[]`` when there is none to read.

    Empty when there is no envelope, when its body is not JSON, and when it holds no
    call with a name. The caller strips the envelope either way, so a body this cannot
    read costs the tool call and never the reply.
    """
    calls: list[dict[str, Any]] = []
    for match in TOOL_CALL_ENVELOPE_PATTERN.finditer(str(text or "")):
        calls.extend(_tool_calls_in_envelope_body(match.group("body")))
    return calls


def _tool_calls_in_envelope_body(body: str) -> list[dict[str, Any]]:
    """Read one envelope body: a single call object, or the array the schema itself shows."""
    try:
        parsed = json.loads(body.strip())
    except (TypeError, ValueError):
        return []
    if isinstance(parsed, dict):
        parsed = [parsed]
    if not isinstance(parsed, list):
        return []
    return [call for call in (_normalize_tool_call(item) for item in parsed) if call]


def _normalize_tool_call(item: object) -> dict[str, Any] | None:
    """One call in the shape the tool runner expects, or None when it names no tool.

    The tool runner indexes into each call, so anything that is not a dict naming a
    tool is dropped here rather than reaching it as an AttributeError.
    """
    if not isinstance(item, dict):
        return None
    tool_name = str(item.get("tool") or "").strip()
    if not tool_name:
        return None
    args = item.get("args")
    return {"tool": tool_name, "args": args if isinstance(args, dict) else {}}


def looks_unusable_reply(reply_text: object) -> bool:
    stripped = str(reply_text or "").strip()
    if not stripped:
        return True

    lowered = stripped.lower().strip(" \t\r\n\"'`.,!?")
    unusable_replies = {
        "i could not generate a response",
        "i couldn't generate a response",
        "i cannot generate a response",
        "unable to generate a response",
        "i'm unable to generate a response",
        "i am unable to generate a response",
    }
    return lowered in unusable_replies


def looks_out_of_character_reply(reply_text: object) -> bool:
    lowered = str(reply_text or "").strip().lower()
    forbidden_phrases = (
        "as an ai",
        "as an ai assistant",
        "i do not possess a personal identity",
        "i don't possess a personal identity",
    )
    return any(phrase in lowered for phrase in forbidden_phrases)


def looks_like_recovery_meta_reply(reply_text: object) -> bool:
    lowered = re.sub(r"\s+", " ", str(reply_text or "").strip().lower())
    if not lowered:
        return False

    meta_markers = (
        "there was no content in the previous reply",
        "there was no incomplete reply to continue from",
        "there was no reply text",
        "there was nothing to continue from",
        "previous message contained no text at all",
        "empty object",
        "empty reply field",
        "complete, albeit empty, json object",
        "only an empty reply field in the json",
        "system returned an empty object",
        "continue from nothing",
        "did not intend to cut you short",
    )
    if any(marker in lowered for marker in meta_markers):
        return True

    return False


def looks_like_generic_handoff_reply(user_text: object, reply_text: object) -> bool:
    user_lowered = re.sub(r"\s+", " ", str(user_text or "").strip().lower())
    reply_lowered = re.sub(r"\s+", " ", str(reply_text or "").strip().lower())
    if not reply_lowered:
        return False

    if re.fullmatch(r"(?:hello|hi|hey|good (?:day|morning|afternoon|evening))(?:[.!?])*", user_lowered):
        return False

    handoff_markers = (
        "tell me your question or your task",
        "you're free to ask whatever you please",
        "i remain at your disposal",
        "pray tell",
        "without interruption",
        "would you like me to assist with something specific",
        "i'm ready to help with automation, spreadsheets, or any factual questions you have",
    )
    if any(marker in reply_lowered for marker in handoff_markers):
        return True

    if re.search(r"\bhow may i assist you(?:\s+(?:today|now))?\b", reply_lowered):
        return True

    return False


def _is_duplicate_event(event_id: object, lock: Any, recent_ids: list[str]) -> bool:
    with lock:
        if event_id in recent_ids:
            return True
        recent_ids.append(event_id)
        return False
````

### `agent.py:5504-6202` — 699 lines, from the first symbol to the last

````python
def normalize_reply_text(value):
    text = str(value or "")
    if not text:
        return ""

    normalized = text.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\r", "\n")
    normalized = normalized.replace("\r\n", "\n").replace("\r", "\n")
    normalized = re.sub(r"\n{3,}", "\n\n", normalized)
    placeholders = {}

    def protect_text(match):
        protected_value = match.group(0)
        trailing_punctuation = ""
        while protected_value and protected_value[-1] in ".,!?;:":
            trailing_punctuation = protected_value[-1] + trailing_punctuation
            protected_value = protected_value[:-1]

        key = f"__PROTECTED_TEXT_{len(placeholders)}__"
        placeholders[key] = protected_value
        return f"{key}{trailing_punctuation}"

    normalized = PROTECTED_TEXT_PATTERN.sub(protect_text, normalized)
    normalized = re.sub(r"([,;:])(?=(?:[\"'([{])?[A-Za-z])", r"\1 ", normalized)

    def normalize_sentence_spacing(match):
        token = match.group("token")
        punctuation = match.group("punct")
        next_chunk = match.group("next")
        if punctuation == "." and token.lower() in COMMON_NON_TERMINAL_ABBREVIATIONS:
            spacing = " "
        else:
            spacing = "  "
        return f"{token}{punctuation}{spacing}{next_chunk}"

    normalized = re.sub(
        r"(?P<token>(?:__PROTECTED_TEXT_\d+__|\b[A-Za-z0-9][A-Za-z0-9.']{0,15}))(?P<punct>[.?!])(?:[ \t]+)?(?P<next>(?:[\"')\]]+[ \t]*)?[A-Za-z])",
        normalize_sentence_spacing,
        normalized,
    )

    for key, original_value in placeholders.items():
        normalized = normalized.replace(key, original_value)

    return normalized.strip()


def markdown_to_ssml(text):
    """Convert markdown formatting to SSML prosody/emphasis tags.
    Used only by Windows TTS which supports SSML.
    
    Mapping:
    - **text** → strong emphasis
    - *text* → moderate emphasis
    - _text_ → reduced pitch (softer)
    
    Note: Square brackets [text] and round brackets (text) are now handled
    separately via segment_text_by_brackets() in synthesize_speech().
    """
    if not text:
        return text
    
    try:
        # Process double asterisks first (** for strong emphasis)
        text = re.sub(r'\*\*([^*]+?)\*\*', r'<emphasis level="strong">\1</emphasis>', text)
        
        # Process single asterisks (* for moderate emphasis)
        text = re.sub(r'\*([^*]+?)\*', r'<emphasis level="moderate">\1</emphasis>', text)
        
        # Process underscores (_ for softer/reduced pitch)
        text = re.sub(r'_([^_]+?)_', r'<prosody pitch="-7%">\1</prosody>', text)
        
        return text
    except Exception:
        return text


_JSON_STRING_ESCAPES = {
    '"': '"',
    "\\": "\\",
    "/": "/",
    "b": "\b",
    "f": "\f",
    "n": "\n",
    "r": "\r",
    "t": "\t",
}


class JsonReplyStreamScanner:
    """Pull the `reply` string out of a JSON reply while it is still arriving.

    `parse_model_json_content` is the authority on what a reply may look like,
    but it needs the whole document. This scanner recognises only the layout the
    prompt actually asks for -- an object whose first key is `reply` (or
    `response`) and whose value is a string -- and gives up (`aborted`) on
    anything else. It never guesses: a layout it cannot follow would otherwise
    produce half a sentence that the final parse then contradicts, so those
    turns simply fall back to today's whole-document behaviour.
    """

    KEYS = ("reply", "response")
    # A prose preamble is not streamable. Bound the hunt so a model that never
    # opens an object cannot grow this buffer without limit.
    MAX_PREAMBLE_CHARS = 256

    def __init__(self):
        self._state = "start"
        self._preamble_chars = 0
        self._key_text = ""
        self._pending_escape = False
        self._unicode_digits = None
        self._pending_high_surrogate = None
        self.reply = ""
        self.aborted = False

    @property
    def done(self):
        return self._state == "done"

    def feed(self, delta):
        """Consume a chunk and return the reply text it completed."""
        emitted = []
        for character in str(delta or ""):
            chunk = self._feed_char(character)
            if chunk:
                emitted.append(chunk)
        return "".join(emitted)

    def finish(self):
        """Close the stream. Returns text still held back for lookahead."""
        held = self._flush_surrogate()
        return held

    def _abort(self):
        self.aborted = True
        self._state = "aborted"
        return self._flush_surrogate()

    def _flush_surrogate(self):
        # A high surrogate with no partner cannot be encoded, so it becomes the
        # usual replacement character rather than an unencodable lone code unit
        # that would break the very JSON response this text is reported in.
        if self._pending_high_surrogate is None:
            return ""
        self._pending_high_surrogate = None
        self.reply += "\ufffd"
        return "\ufffd"

    def _feed_char(self, character):
        state = self._state
        if state in ("done", "aborted"):
            return ""

        if state == "start":
            if character.isspace():
                return ""
            self._count_preamble()
            if character == "`":
                self._state = "fence"
                return ""
            if character == "{":
                self._state = "object"
                return ""
            return self._abort()

        if state == "fence":
            # ``` or ```json -- the tag is optional and never contains a brace.
            if character == "{":
                self._state = "object"
                return ""
            if character in "`-_" or character.isalnum() or character.isspace():
                return self._count_preamble()
            return self._abort()

        if state == "object":
            if character.isspace():
                return ""
            if character == '"':
                self._key_text = ""
                self._state = "key"
                return ""
            return self._abort()

        if state == "key":
            if character == '"':
                if self._key_text not in self.KEYS:
                    return self._abort()
                self._state = "colon"
                return ""
            if character.isalpha() or character == "_":
                self._key_text += character
                if not any(key.startswith(self._key_text) for key in self.KEYS):
                    return self._abort()
                return ""
            return self._abort()

        if state == "colon":
            if character.isspace():
                return ""
            if character == ":":
                self._state = "value"
                return ""
            return self._abort()

        if state == "value":
            if character.isspace():
                return ""
            if character == '"':
                self._state = "string"
                return ""
            # A non-string reply is not worth streaming, and guessing at it
            # would only produce text the final parse disagrees with.
            return self._abort()

        # state == "string"
        if self._pending_escape:
            self._pending_escape = False
            return self._feed_escape(character)
        # Still collecting the four digits of a \uXXXX escape.
        if self._unicode_digits is not None:
            return self._feed_escape(character)
        if character == "\\":
            self._pending_escape = True
            return ""
        if character == '"':
            self._state = "done"
            return self._flush_surrogate()
        # Only another \uXXXX escape can complete a held high surrogate, so
        # ordinary text means it never will be -- and it has to be written out
        # here, not at the end, or the reply reads with the character misplaced.
        if self._pending_high_surrogate is not None:
            flushed = self._flush_surrogate()
            self.reply += character
            return flushed + character
        self.reply += character
        return character

    def _count_preamble(self):
        self._preamble_chars += 1
        if self._preamble_chars > self.MAX_PREAMBLE_CHARS:
            return self._abort()
        return ""

    def _feed_escape(self, character):
        if self._unicode_digits is not None:
            if character in "0123456789abcdefABCDEF":
                self._unicode_digits += character
                if len(self._unicode_digits) < 4:
                    return ""
                digits = self._unicode_digits
                self._unicode_digits = None
                return self._emit_code_point(int(digits, 16))
            # Malformed \u escape. Keep what was written and carry on with this
            # character rather than losing the rest of the reply.
            digits = self._unicode_digits
            self._unicode_digits = None
            literal = "\\u" + digits
            self.reply += literal
            return literal + self._feed_char(character)

        if character == "u":
            self._unicode_digits = ""
            return ""

        # Any other escape forecloses on a held high surrogate.
        flushed = self._flush_surrogate()
        mapped = _JSON_STRING_ESCAPES.get(character)
        if mapped is None:
            # JSON forbids escapes like \q. Dropping the backslash loses less
            # than aborting a reply the model is halfway through.
            mapped = character
        self.reply += mapped
        return flushed + mapped

    def _emit_code_point(self, code):
        high = self._pending_high_surrogate
        if 0xD800 <= code <= 0xDBFF:
            self._pending_high_surrogate = code
            if high is not None:
                self.reply += "\ufffd"
                return "\ufffd"
            return ""
        if 0xDC00 <= code <= 0xDFFF:
            self._pending_high_surrogate = None
            if high is None:
                self.reply += "\ufffd"
                return "\ufffd"
            paired = chr(0x10000 + ((high - 0xD800) << 10) + (code - 0xDC00))
            self.reply += paired
            return paired
        self._pending_high_surrogate = None
        resolved = chr(code)
        if high is not None:
            resolved = "\ufffd" + resolved
        self.reply += resolved
        return resolved


# Markup the client and `segment_text_by_brackets` both read as control text. A
# split inside an unbalanced run of it would change what gets spoken.
_SPLITTER_MARKUP_PAIRS = (("{", "}"), ("[", "]"), ("(", ")"), ("*", "*"), ("_", "_"))
_SPLITTER_TERMINATORS = ".!?…"
# Two or more single letters separated by dots: "U.S", "e.g", "a.m". A lone
# trailing initial ("plan B.") is a real sentence ending, so one group is not
# enough to call it an acronym.
_DOTTED_ACRONYM_RE = re.compile(r"^(?:[A-Za-z]\.){2,}[A-Za-z]?$")
# A trailing run of these is a soft pause: the last place to cut a sentence that
# has grown past MAX_SENTENCE_CHARS without ever reaching a full stop. A comma is
# deliberately absent: cutting there hands the listener a clip boundary where the
# sentence has not ended, and the gap between clips is heard as an over-long
# pause after the comma. Such a sentence falls back to the word-boundary cut
# instead, the same place the client's own chunker cuts.
_SPLITTER_SOFT_BREAKS = ";:—–"


class SentenceSplitter:
    """Chop streamed reply text into speakable sentences.

    Only text that is safe to speak on its own is released. Anything ambiguous
    waits for more input: a split inside `{tone}`, `[scene]`, `(aside)`, `*` or
    `_` markup, inside `3.5`, inside `Mr.`, or inside a sentence too short to
    stand alone ("Ha. Yes.") is held back until the text resolves it.
    """

    MIN_SENTENCE_CHARS = 12
    MAX_SENTENCE_CHARS = 240

    def __init__(self):
        self._buffer = ""

    def feed(self, delta):
        """Add a chunk and return every sentence it completed."""
        self._buffer += str(delta or "")
        return self._extract()

    def flush(self):
        """Release whatever is left, which is the end of the reply."""
        remainder = self._buffer.strip()
        self._buffer = ""
        return [remainder] if remainder else []

    def _extract(self):
        sentences = []
        while True:
            boundary = self._next_boundary()
            if boundary is not None:
                sentence = self._buffer[:boundary].strip()
                self._buffer = self._buffer[boundary:]
                if sentence:
                    sentences.append(sentence)
                continue

            if len(self._buffer) <= self.MAX_SENTENCE_CHARS:
                return sentences

            forced = self._forced_boundary()
            if forced is None:
                return sentences
            sentence = self._buffer[:forced].strip()
            if not sentence:
                return sentences
            sentences.append(sentence)
            self._buffer = self._buffer[forced:]

    def _next_boundary(self):
        for index, character in enumerate(self._buffer):
            if character not in _SPLITTER_TERMINATORS:
                continue
            end = index + 1
            while end < len(self._buffer) and self._buffer[end] in _SPLITTER_TERMINATORS:
                end += 1
            # Wait for whatever follows the terminator: "3." is a number, not an
            # ending, and end-of-buffer is never a safe place to cut.
            if end >= len(self._buffer):
                return None
            if not self._buffer[end].isspace():
                continue
            if end - index < 3 and not self._is_sentence_end(index):
                continue
            if end < self.MIN_SENTENCE_CHARS:
                continue
            if not self._markup_balanced(self._buffer[:end]):
                continue
            return end
        return None

    def _is_sentence_end(self, index):
        # A single "." is also how an abbreviation, an initial or a decimal
        # point is written, so it only ends a sentence when the token before it
        # is not one of those.
        if self._buffer[index] != ".":
            return True
        return not self._is_non_terminal_period(index)

    def _is_non_terminal_period(self, index):
        if self._buffer[index] != ".":
            return False
        following = index + 1
        if following < len(self._buffer) and self._buffer[following].isdigit():
            if index > 0 and self._buffer[index - 1].isdigit():
                return True
        token_match = re.search(r"[A-Za-z][A-Za-z.]*$", self._buffer[: index + 1])
        if not token_match:
            return False
        token = token_match.group(0).lower().rstrip(".")
        if token in COMMON_NON_TERMINAL_ABBREVIATIONS:
            return True
        # Dotted acronyms and initials -- "U.S.", "e.g.", "a.m." -- are not in
        # the shared abbreviation list, and adding them there would change how
        # every other caller reads text.
        return bool(_DOTTED_ACRONYM_RE.match(token))

    def _markup_balanced(self, text):
        for opening, closing in _SPLITTER_MARKUP_PAIRS:
            if opening == closing:
                # `*action*` and `_emphasis_` use the same character twice, so
                # the only thing to check is that the run is closed.
                if text.count(opening) % 2:
                    return False
            elif text.count(opening) != text.count(closing):
                return False
        return True

    def _forced_boundary(self):
        limit = self.MAX_SENTENCE_CHARS
        window = self._buffer[:limit]
        for index in range(len(window) - 1, 0, -1):
            if window[index] not in _SPLITTER_SOFT_BREAKS:
                continue
            if not self._markup_balanced(window[: index + 1]):
                continue
            return index + 1
        for index in range(len(window) - 1, 0, -1):
            if window[index].isspace() and self._markup_balanced(window[:index]):
                return index
        return None


def _streamed_prefix_length(reply, spoken):
    """How much of `reply` the already-spoken `spoken` covers, or None.

    Whitespace is not compared: the splitter strips each sentence, so runs of
    spaces and newlines legitimately differ between the two. Everything else
    must match exactly, because anything else means the text the user heard is
    not the text being returned.
    """
    reply_index = 0
    reply_length = len(reply)
    for character in spoken:
        if character.isspace():
            continue
        while reply_index < reply_length and reply[reply_index].isspace():
            reply_index += 1
        if reply_index >= reply_length or reply[reply_index] != character:
            return None
        reply_index += 1
    return reply_index


def split_reply_after_stream(reply, spoken):
    """Return `(remaining, replaced)` for a reply that was partly streamed.

    `remaining` is the part the user has not heard. `replaced` reports that the
    streamed text is not a prefix of the final reply at all -- the reply was
    rewritten after it started being spoken, so the audio and the transcript
    disagree and the count is worth watching.
    """
    reply = str(reply or "")
    spoken = str(spoken or "")
    prefix_length = _streamed_prefix_length(reply, spoken)
    if prefix_length is None:
        return reply, bool(spoken.strip())
    return reply[prefix_length:].lstrip(), False


def strip_streamed_prefix(reply, spoken):
    """Drop a leading copy of text the user has already heard."""
    reply = str(reply or "")
    prefix_length = _streamed_prefix_length(reply, str(spoken or ""))
    if prefix_length is None:
        return reply
    return reply[prefix_length:].lstrip()


class ReplyStreamPublisher:
    """Turn a generated token stream into sentences the browser can speak.

    Sits between the model and the chat job: `feed` decodes the reply out of the
    JSON as it arrives and publishes each finished sentence, and `finish`
    reports what was actually spoken so the caller can work out how much of the
    final reply the user has already heard.
    """

    def __init__(self, publish=None):
        self._publish = publish
        self._scanner = JsonReplyStreamScanner()
        self._splitter = SentenceSplitter()
        self.spoken = []
        self.published = 0
        self.aborted = False

    def feed(self, piece):
        if self._scanner.aborted or self._scanner.done:
            return
        text = self._scanner.feed(piece)
        if self._scanner.aborted:
            # The reply is not laid out the way the prompt asks, so nothing can
            # safely be spoken early. The turn falls back to whole-reply audio.
            self.aborted = True
            return
        self._emit(self._splitter.feed(text))

    def finish(self):
        self._emit(self._splitter.flush())
        return self.spoken_text()

    def spoken_text(self):
        return " ".join(self.spoken)

    def _emit(self, sentences):
        for sentence in sentences:
            cleaned = str(sentence or "").strip()
            if not cleaned:
                continue
            self.spoken.append(cleaned)
            self.published += 1
            if self._publish is not None:
                self._publish(cleaned)


# The keys that make a parsed dict a reply document rather than a dict that
# merely appeared in one. `replies` and `response` are the documented aliases the
# parser below already understands.
REPLY_DOCUMENT_KEYS = ("reply", "replies", "response", "tool_calls")


def _parse_structured_candidate(candidate):
    """Parse one reply candidate as JSON, then as a Python literal dict.

    A model shown the JSON schema still answers with a Python dict repr often
    enough to matter -- single quotes, a trailing comma, ``None`` for null -- and
    every `json.loads` candidate failed on it. The repr was then returned whole as
    the reply text with its tool calls dropped, so the turn spoke the model's own
    syntax at the user and silently did nothing.

    A literal is accepted only when it carries a key a reply document is made of.
    JSON is read exactly as it was before, so nothing that parsed then parses
    differently now, and the stricter rule here is what stops `literal_eval` from
    reading a brace-bearing sentence as a reply document and emptying the reply.
    `literal_eval` executes nothing.
    """
    try:
        parsed = json.loads(candidate)
    except Exception:
        parsed = None
    if isinstance(parsed, dict):
        return parsed

    try:
        literal = ast.literal_eval(candidate)
    except Exception:
        return None
    if isinstance(literal, dict) and any(key in literal for key in REPLY_DOCUMENT_KEYS):
        return literal
    return None


def _parse_model_reply_document(content):
    text = str(content or "").strip()
    if not text:
        return {"reply": "", "tool_calls": []}

    candidates = [text]

    if text.startswith("```"):
        fenced = re.sub(r"^```(?:json)?\s*|\s*```$", "", text, flags=re.IGNORECASE | re.DOTALL).strip()
        if fenced:
            candidates.append(fenced)

    json_match = re.search(r"\{.*\}", text, flags=re.DOTALL)
    if json_match:
        candidates.append(json_match.group(0))

    for candidate in candidates:
        parsed = _parse_structured_candidate(candidate)
        if parsed is None:
            continue

        if "reply" not in parsed and isinstance(parsed.get("response"), str):
            parsed["reply"] = parsed.get("response", "")
        # A model may answer in several distinct beats. Then the segments are
        # the structured truth, `reply` becomes their join, and every existing
        # caller keeps working unchanged. `reply_segments` is only set when
        # there is more than one, so its presence is itself the signal that
        # this turn wants more than one bubble.
        segments = [
            normalize_reply_text(segment)
            for segment in extract_reply_segments(parsed.get("replies"))
        ]
        segments = [segment for segment in segments if segment]
        if len(segments) > 1:
            parsed["reply_segments"] = segments
            parsed["reply"] = normalize_reply_text("\n\n".join(segments))
        else:
            # A single `replies` entry only fills in a missing `reply`. When
            # both are present the explicit `reply` wins, as it always has --
            # otherwise a stray one-element array would silently override it.
            parsed["reply"] = normalize_reply_text(
                str(parsed.get("reply") or "").strip()
                or (segments[0] if segments else "")
            )
        parsed.setdefault("tool_calls", [])
        return parsed

    reply_match = re.search(r'"reply"\s*:\s*"((?:\\.|[^"\\])*)"', text, flags=re.DOTALL)
    if reply_match:
        encoded_reply = reply_match.group(1)
        try:
            decoded_reply = json.loads(f'"{encoded_reply}"')
        except Exception:
            decoded_reply = encoded_reply
        return {"reply": normalize_reply_text(decoded_reply), "tool_calls": []}

    partial_reply_match = re.search(r'"reply"\s*:\s*"(?P<reply>.*)$', text, flags=re.DOTALL)
    if partial_reply_match:
        partial_reply = partial_reply_match.group("reply")
        partial_reply = re.sub(r'"\s*,?\s*"tool_calls"\s*:\s*\[.*$', "", partial_reply, flags=re.DOTALL)
        partial_reply = re.sub(r'"\s*}*\s*$', "", partial_reply, flags=re.DOTALL)
        partial_reply = partial_reply.replace('\\r\\n', '\n').replace('\\n', '\n').replace('\\r', '\n')
        partial_reply = partial_reply.replace('\\"', '"').replace('\\\\', '\\')
        return {"reply": normalize_reply_text(partial_reply), "tool_calls": []}

    return {"reply": normalize_reply_text(text), "tool_calls": []}


def reply_without_prose_tool_calls(text):
    """Text with any prose tool-call envelope removed -- byte-identical when there is none.

    Renormalising only when something was removed is what keeps a reply that never had
    an envelope exactly as it was, rather than passing it through the normaliser twice.
    """
    settled = strip_tool_call_envelope(text)
    return normalize_reply_text(settled) if settled != text else text


def settle_prose_tool_calls(parsed):
    """Move any prose tool-call envelope out of the reply and into ``tool_calls``.

    The model is asked for tool calls as a JSON field and sometimes writes the envelope
    as prose instead, borrowing the tone-tag syntax. When that happens the call would
    otherwise be lost and the envelope spoken aloud, so it is recovered here.
    """
    reply = str(parsed.get("reply") or "")
    recovered = tool_calls_in_envelope(reply)
    settled = reply_without_prose_tool_calls(reply)
    if settled != reply:
        # A marker reached the reply, which by construction means the model wrote the
        # envelope as prose and the call was about to be lost. Never silent again.
        app.logger.warning(
            "Prose tool-call envelope in the reply: tools=%s text=%r",
            [call.get("tool") for call in recovered],
            reply[:300],
        )
        parsed["reply"] = settled
    if parsed.get("reply_segments"):
        segments = [
            settled_segment
            for settled_segment in (
                reply_without_prose_tool_calls(segment)
                for segment in parsed["reply_segments"]
            )
            if settled_segment
        ]
        if len(segments) > 1:
            # `reply` is the segments' join, as the parse body defines it, so it
            # is rebuilt from the settled segments rather than stripped in place:
            # otherwise the join keeps a hole where an envelope was and disagrees
            # with the bubbles the client actually shows.
            parsed["reply_segments"] = segments
            parsed["reply"] = normalize_reply_text("\n\n".join(segments))
        else:
            # Its presence is the signal for more than one bubble, so a segment
            # that was nothing but an envelope leaves the turn single-bubble
            # rather than adding an empty one to read and speak aloud.
            parsed.pop("reply_segments", None)
    if recovered and not parsed.get("tool_calls"):
        parsed["tool_calls"] = recovered
    return parsed


def parse_model_json_content(content):
    """Turn raw model text into a reply and its tool calls.

    The model is asked for tool calls as a JSON field and sometimes writes them as prose
    instead, wrapping the same JSON in a ``{tool_calls}`` tag borrowed from the tone-tag
    syntax. That is settled here, at the one entry point every parse goes through, so no
    reply can carry an envelope on to storage, the client, or the speech path.
    """
    return settle_prose_tool_calls(_parse_model_reply_document(content))
````

### The test list, module by module

136 tests in the four modules. Port every one; the name says what it pins, and the source of each
test is the reference module beside the app (`D:\Entity-In-The-Shed\tests\`), which this brief does
not paste — the session may read those four test files, and only those, to get the assertions.

### test_reply_utils.py
- test_teaser_phrases
- test_the_reported_failure
- test_promise_of_a_look
- test_courtesy_is_not_a_teaser
- test_long_answer_containing_a_teaser_phrase_is_not_a_teaser
- test_boundary_is_the_word_count
- test_plain_reply
- test_blank_and_none
- test_context_and_intent
- test_context_only
- test_intent_only
- test_blank_and_none
- test_blank_and_none
- test_done_reason_length
- test_done_reason_stop_not_incomplete
- test_trailing_colon_or_ellipsis
- test_long_multiline_without_terminal_punctuation
- test_poem_teaser_phrase
- test_whitespace_only
- test_complete_sentence
- test_returns_last_user_content_stripped
- test_no_user_messages
- test_none_and_empty
- test_stub_reply
- test_non_generative_request
- test_multiline_reply_not_stub
- test_long_reply_not_stub
- test_content_marker_not_stub
- test_blank_inputs
- test_completion_starts_with_partial
- test_overlap_join
- test_no_overlap_completion_starts_with_capital
- test_no_overlap_word_join
- test_empty_completion_returns_partial_stripped
- test_empty_partial_returns_completion
- test_equal_replies
- test_absent_field_yields_no_segments
- test_a_single_segment_is_still_returned
- test_strips_and_drops_empties
- test_drops_non_strings
- test_caps_at_the_maximum
- test_bare_string_is_one_segment
- test_tuple_accepted
- test_does_not_normalise
- test_known_phrases
- test_normal_reply
- test_blank
- test_forbidden_phrases
- test_normal_reply
- test_blank
- test_meta_markers
- test_normal_reply
- test_blank
- test_handoff_markers
- test_assist_regex
- test_greeting_user_not_handoff
- test_normal_reply
- test_blank_reply
- test_first_seen_false_then_true
- test_new_ids_appended
- test_maxlen_eviction
- test_none_id
- test_removes_a_closed_envelope
- test_removes_a_lone_tag
- test_tolerates_spacing_and_case_in_the_tag
- test_removes_every_envelope
- test_leaves_tone_tags_alone
- test_empty_input
- test_single_object
- test_array_form
- test_every_envelope_is_read
- test_args_that_are_not_an_object_become_empty
- test_a_call_with_no_name_is_dropped
- test_a_non_dict_entry_is_dropped
- test_a_body_that_is_not_json_yields_nothing
- test_a_json_scalar_yields_nothing
- test_no_envelope
- test_a_lone_tag_carries_nothing
### test_reply_stream_scanner.py
- test_whole_document_in_one_chunk
- test_character_by_character_is_the_same_as_one_chunk
- test_response_key_is_accepted_like_the_parser_does
- test_whitespace_inside_the_object_is_tolerated
- test_fenced_json_is_tolerated
- test_escapes_are_decoded
- test_escape_split_across_chunks
- test_unicode_escape_split_mid_digits
- test_surrogate_pair_split_across_chunks_forms_one_code_point
- test_lone_high_surrogate_becomes_a_replacement_character
- test_trailing_lone_surrogate_is_flushed_at_end_of_stream
- test_reply_stops_at_the_closing_quote
- test_tool_calls_first_aborts_without_emitting
- test_prose_preamble_aborts
- test_bare_text_aborts
- test_non_string_reply_aborts
- test_unbounded_prose_preamble_aborts_instead_of_growing
- test_abort_is_terminal
- test_unknown_escape_keeps_the_character
- test_empty_and_none_chunks_are_ignored
### test_reply_stream_publisher.py
- test_publishes_finished_sentences_in_order
- test_publishes_before_the_document_is_complete
- test_an_aborted_layout_publishes_nothing
- test_publishing_stops_after_an_abort
- test_the_remainder_after_the_stream_is_reported
- test_a_missing_publish_callback_is_tolerated
- test_published_count_matches_the_list
- test_remaining_is_the_part_not_yet_spoken
- test_nothing_remaining_when_the_whole_reply_was_spoken
- test_nothing_spoken_means_the_whole_reply_remains
- test_whitespace_differences_do_not_count_as_a_replacement
- test_a_rewritten_reply_is_reported_as_replaced
- test_a_replacement_is_not_reported_when_nothing_was_spoken
- test_a_repeated_preface_is_dropped
- test_a_different_follow_up_is_left_alone
- test_nothing_spoken_leaves_the_reply_alone
- test_a_reply_that_is_only_the_preface_becomes_empty
### test_sentence_splitter.py
- test_splits_on_sentence_terminators
- test_chunking_does_not_change_the_result
- test_abbreviations_do_not_end_a_sentence
- test_dotted_acronyms_do_not_end_a_sentence
- test_decimals_do_not_end_a_sentence
- test_version_like_numbers_still_end_a_sentence
- test_ellipsis_ends_a_sentence
- test_short_fragments_merge_with_the_next_sentence
- test_no_split_inside_a_tone_tag
- test_no_split_inside_a_scene_bracket
- test_no_split_inside_an_aside
- test_an_unclosed_opener_holds_the_boundary_but_not_the_reply
- test_no_split_inside_emphasis
- test_trailing_text_without_a_terminator_arrives_at_flush
- test_flush_of_a_dangling_terminator_emits_it
- test_flush_of_whitespace_only_emits_nothing
- test_long_unbroken_text_is_not_cut_on_a_comma
- test_long_text_is_still_cut_on_a_semicolon
- test_long_text_without_any_punctuation_is_still_bounded
- test_every_sentence_is_non_empty_and_stripped
- test_reassembled_output_keeps_all_the_words

### A note on the two modules this brief does not paste

`tests/test_glm4.py` and `tests/test_glm4_followup.py` are fixtures: they build the reply documents
that `test_reply_utils` and `test_api_chat` feed in. Read them for the documents, add those documents
to `document.json`, and do not port the modules as suites.

### What this brief deliberately leaves to 4f-2 and 4f-3

- `run_agent`, the three guards (`TurnGate`, the job TTL + sentence cap, the duplicate-event guard),
  `persist_exchange_reply`, and the recovery path of `agent.py:7760-8417` — 4f-2.
- `POST /api/chat`, the job status route, the envelopes, and Stage 4's exit gate — 4f-3.
- The `shed-chat` items this sub-slice creates are the ones 4f-2 will call: `ReplyStreamPublisher`,
  `parse_model_json_content`, `RecentEvents`, and `looks_incomplete` / `looks_unusable_reply` /
  `merge_completed_reply`. Keep them `pub`; a crate that hides them makes 4f-2 reopen this slice.
