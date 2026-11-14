# Slice 4f brief — the chat turn, and the split it needs before it starts

Hand this file to the session as its task. **This file is a scoping brief, not an implementation
brief.** It fixes the split, the destination crates, the guard rows and Stage 4's exit gate. Each
sub-slice gets its own brief — with its own source pasted in — written the same way as
`SLICE-4d-BRIEF.md` and `SLICE-4e-BRIEF.md` when that sub-slice starts.

**Do not read `agent.py`.** Do not start writing `shed-chat` from this file alone.

**Every path in this brief is relative to `D:\Entity_In_The_Shed`.** The files tool is rooted at
`D:\` and the terminal tool starts in `D:\`: pass the full `D:\Entity_In_The_Shed\...` path to the
files tool, and start every terminal command with `cd /d D:\Entity_In_The_Shed && `.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2, **line 377**: `| **4f** | The chat turn: reply shaping,
  scanner, splitter, publisher, run_agent, TurnGate, POST /api/chat + job route | agent.py:5504-6202,
  7760-8417, 9389-9510 | test_api_chat, test_reply_* (4), test_sentence_splitter, test_glm4* |
  Stage 4's own exit gate: a chat turn streams sentences to the frozen client, and the busy path
  reproduces under two simultaneous requests. **Expect this to split again when it is reached** |`.
- `main` is the branch. 4a–4c and 4d are committed; 4e is the slice in flight.

## Why it splits — the numbers

| region | lines |
|---|---|
| `agent.py:5504-6202` (reply text, scanner, splitter, publisher, parsers) | 699 |
| `agent.py:7760-8417` (persist, tool-call guards, recovery, `run_agent`, completeness, duplicate events) | 658 |
| `agent.py:9389-9510` (the job status route and `POST /api/chat`) | 122 |

plus `utils/reply.py` (14.5 KB) and `utils/prompt_cache.py`, and these test modules:

| module | tests |
|---|---|
| `tests/test_reply_utils.py` | 78 |
| `tests/test_reply_recovery.py` | 147 |
| `tests/test_api_chat.py` | 51 |
| `tests/test_reply_stream_scanner.py` | 20 |
| `tests/test_sentence_splitter.py` | 21 |
| `tests/test_reply_stream_publisher.py` | 17 |
| `tests/test_glm4.py`, `tests/test_glm4_followup.py` | fixtures for the reply parsers |

That is ~1,480 source lines and ~334 tests — three to four times slice 4e. One turn cannot hold it,
and the plan says so itself. The split is by **layer**, so each sub-slice is independently green:

## The three sub-slices

### 4f-1 — reply shaping, scanning, splitting, publishing (pure, no I/O)

- **Source**: `utils/reply.py` in full, plus `agent.py:5504-6202` —
  `normalize_reply_text`, `markdown_to_ssml`, `JsonReplyStreamScanner` (class), `SentenceSplitter`
  (class), `_streamed_prefix_length`, `split_reply_after_stream`, `strip_streamed_prefix`,
  `ReplyStreamPublisher` (class), `_parse_structured_candidate`, `_parse_model_reply_document`,
  `reply_without_prose_tool_calls`, `settle_prose_tool_calls`, `parse_model_json_content`.
- **Crate**: `shed-chat` — created here (D4: created on demand), registered in
  `[workspace.dependencies]` with its consumers, and never given a dependency the layer map forbids
  (`shed-chat → config, json, text, models, llm, prompts, tools, mcp, conversation`).
- **Tests**: `test_reply_utils` (78), `test_reply_stream_scanner` (20), `test_reply_stream_publisher`
  (17), `test_sentence_splitter` (21), plus the `test_glm4*` fixtures that pin the reply parsers.
- **Done when**: those pass, and the module opens no file, no socket and starts no thread. The
  scanner and splitter are fed by callers; they must not read the model themselves.
- **D8 fixtures**: the glm4 and reply-shaping golden fixtures are captured in this sub-slice, by
  importing the pure Python modules in a subprocess and writing the JSON — **never** by booting the
  reference app or binding port 5005.

### 4f-2 — the agent loop, recovery, and the three guards

- **Source**: `agent.py:7760-8417` — `persist_exchange_reply`,
  `build_protected_tool_auth_reply`, `build_forced_web_search_tool_calls`,
  `execute_mctrader_tool_calls`, `recover_agent_reply`, `build_agent_follow_up_messages`,
  `run_agent`, `ensure_complete_reply`, `ensure_usable_reply`, `is_duplicate_event`.
- **Crate**: `shed-chat`.
- **Tests**: `test_reply_recovery` (147).
- **Guards**: this is the sub-slice that turns `docs/GUARD_CATALOGUE.md`'s Stage-4 rows from
  `pending` to `ported`, each naming its proving test: **`shed-chat::TurnGate`** (single conversation
  turn slot / busy reply), the **chat job TTL + sentence cap**, and the **duplicate-event guard**
  (`run_agent` region). Those rows are what `shed-gates::guards`' honesty gate is waiting for — the
  first ported runtime guard is the point where that gate starts meaning something.
- **Done when**: the 147 pass; the busy path reproduces from two concurrent turns; the turn slot is
  a named owner, not a bare `std::thread` spawned per request.

### 4f-3 — the HTTP surface, and Stage 4's exit gate

- **Source**: `agent.py:9389-9510` — `browser_chat_job_status`, `browser_chat` — plus the envelopes
  they use. `chat_job_not_found` (`agent.py:9409`, plan §8.1) is that envelope's **first reachable
  caller** in the clone, so it lands here.
- **Crate**: `shed-http` for the route and the envelopes, `shed-server` for the wiring.
- **Tests**: `test_api_chat` (51).
- **Stage 4 exit gates** (both must run against the built binary, not a mock):
  1. a chat turn **streams sentences** to the frozen client;
  2. the **busy path** reproduces under two simultaneous requests (one gets the busy reply, the
     other completes; no interleaved state).
- **Done when**: both gates pass, and every Stage-4 row in `GUARD_CATALOGUE.md` reads `ported`.

## Decisions already taken — do not re-derive

1. **Three sub-slices, in this order (4f-1 → 4f-2 → 4f-3), one per turn, each ending green and
   committed.** Do not attempt 4f in one turn, and do not reorder: the HTTP surface cannot be tested
   before the reply layer and the loop exist.
2. **Each sub-slice's brief pastes its own source** (`utils/reply.py` + `agent.py:5504-6202` for
   4f-1; `7760-8417` for 4f-2; `9389-9510` plus the job/envelope definitions for 4f-3). This file
   does not paste any, and a session must not read `agent.py` to make up the difference.
3. **Fixtures are captured, not booted.** D8's golden fixtures (chat turns, glm4 output, job
   `sentences[]` progress) come from importing the pure Python modules in a subprocess. The standing
   rule against booting the reference app or binding 5005 is absolute.
4. **The frozen client already exists.** Before writing any client stub, locate it —
   search `crates/`, `docs/` and the existing `shed-http` byte-exact tests for the frozen contract
   and use it. A second frozen client beside the real one is worse than none.
5. **`shed-chat` is created in 4f-1** with `Cargo.toml` entry, workspace dependency list and layer
   position as above. `unused_crate_dependencies` means a crate with no callers fails the gate: 4f-1
   is the crate's first caller, so its public API must be exercised by its own tests.
6. **The guards belong to 4f-2, not 4f-1.** The reply layer is stateless by design; the turn slot,
   the TTL/cap and the duplicate-event guard are all `run_agent`-region behaviour.
7. **`persist_exchange_reply` and the tool-call guards keep their source order and names.** The
   recovery path is the thing the 147 tests measure; do not "improve" the order of `ensure_complete_reply`
   and `ensure_usable_reply`.
8. **No new dependency without the workspace list**, and none of the layer map's forbidden edges —
   in particular `shed-chat` may use `shed-llm`, `shed-tools` and `shed-mcp`, but `shed-http` may not
   be used by it (the composition root is above, not below).

## What is NOT in 4f

- The idle housekeeper and profile workers (4e), the record store and history assembly (4d).
- The tool registry and its implementations, the MCP servers, TTS/voice, the mind/entity crates:
  other stages. 4f calls them through their public APIs, it does not build them.
- Job persistence beyond what `POST /api/chat` and the job route need.

## How to start

When 4e is committed, write `TODO/SLICE-4f1-BRIEF.md` in the same shape as 4d/4e: header, mission,
the source pasted verbatim (verified byte-identical against `utils/reply.py` and
`agent.py:5504-6202` — write a small checker rather than trusting the paste), the test list from
`test_reply_utils`/`test_reply_stream_scanner`/`test_reply_stream_publisher`/`test_sentence_splitter`,
the gate, and the stop condition. **Writing that brief is the turn.** Do not start 4f-1's code from
this file.

## Gate (every sub-slice, before its commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # report the total you get
cmd /c "npm test 2>&1"                      # green, unchanged
```

## Stop condition — read this last, obey it exactly

1. **Asked to "do 4f" with no sub-slice brief on disk** → write `SLICE-4f1-BRIEF.md` (that is one
   turn's work), or stop and say the brief is missing. Never reconstruct the source from `agent.py`.
2. **Gate red after two honest fix attempts** in any sub-slice → stop and report the output. Do not
   widen that sub-slice, and do not merge two sub-slices to "save a turn".

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
