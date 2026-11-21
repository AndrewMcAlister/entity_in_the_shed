# Guard Catalogue (principle 23)

One entry per guard: what it protects, where it lives, what fires and clears
it, its config key, and the tests that pin it. `crates/shed-gates` will fail
on a stale entry location or a known guard with no entry — that check is due
now that slice 4f-1 has ported the first runtime guards, so until it lands the
entries below are the source of truth by hand-check.

Entry format:

```
## <Guard name>
- Owner: <crate>::<module>::<function>
- Gates: <what action is refused/degraded when it fires>
- Fires: <condition>   Clears: <condition>
- Config: <ENV_KEY(s) + defaults>
- Tests: <test names>
- Note: <~5 lines of why this exists>
```

Statuses: **pending (Stage N)** = exists in the Python source, not yet
ported; **ported** = entry filled with live Rust locations and tests.

## Known guards (from the Python source; entries fill in as stages port them)

| Guard | Owner lands in | Status |
|---|---|---|
| Single conversation turn slot (busy reply) | `shed-chat::TurnGate` | pending (Stage 4) |
| Chat job TTL + sentence cap | `shed-chat` | pending (Stage 4) |
| Persona enforcement on replies (`enforce_persona_reply`, `agent.py:4886`) | `shed-conversation` | pending (Stage 4) |
| Tool auth gating + arming (`/api/mcp/auth`) | `shed-mcp` | pending (Stage 5) |
| Trade confirmation + code expiry + price-move refusal | `shed-tools` | pending (Stage 5) |
| Private-address block for web fetch (`is_public_web_url`) | `shed-tools` | pending (Stage 5) |
| SSE reconnect throttle (`entity_reconnect_too_fast`) | `shed-entity` | pending (Stage 7) |
| SSE subscriber cap (`entity_stream_full`) + queue-drop-oldest | `shed-entity` | pending (Stage 7) |
| Entity stream disabled flag | `shed-entity` | pending (Stage 7) |
| Speech budget caps (`ENTITY_MAX_UTTERANCES_PER_HOUR/DAY`) | `shed-mind` | pending (Stage 7) |
| Quiet hours | `shed-mind` | pending (Stage 7) |
| Repeat guard (`ENTITY_REPEAT_GUARD_LINES`) | `shed-mind` | pending (Stage 7) |
| Min utterance gap (`ENTITY_MIN_UTTERANCE_GAP_SECONDS`) | `shed-mind` | pending (Stage 7) |
| Outbox TTLs + drain-until-ack | `shed-entity` | pending (Stage 7) |
| The ~20 slow-loop vetoes (`utils/entity_loop.py`) | `shed-mind` (named guard fns) | pending (Stage 7) |
| Grounding checks (market data vs reply claims) | `shed-mind` | pending (Stage 7) |
| Duplicate-event guard (`run_agent` region) | `shed-chat` | pending (Stage 4) |
| Reply completeness/usability recovery | `shed-chat::quality` | **ported** |
| Prose tool-call envelope (`settle_prose_tool_calls`) | `shed-chat::document` | **ported** |

## Entries

## Reply completeness/usability recovery
- Owner: `shed-chat::quality::looks_unusable_reply`, `shed-chat::quality::looks_incomplete`,
  `shed-chat::quality::merge_completed_reply` (all three from `utils/reply.py`)
- Gates: whether a finished turn is spoken at all. An unusable reply is replaced
  before the listener hears it; an incomplete one is recovered from the streamed
  partial instead of being spoken cut off.
- Fires: a reply that is empty or is one of the app's own failure lines arriving
  back as an answer, quotes and terminal punctuation stripped first
  (`looks_unusable_reply`); a reply that is empty, or was cut off by the token
  limit (`_meta.done_reason == "length"`), or ends on a colon or an ellipsis, or
  runs long with no terminal punctuation, or is a teaser phrase
  (`looks_incomplete`).   Clears: a reply that passes both tests, which is
  spoken untouched.
- Config: none — the source has no env key for these.
- Tests: `crates/shed-chat/tests/quality.rs`, `fixture_looks_unusable_reply`,
  `fixture_looks_incomplete`, `fixture_merge_completed_reply`
  (rows captured from the running source).
- Note: the model sometimes echoes one of the app's own error lines back as if
  it were an answer, or stops mid-sentence when the token limit cuts it off.
  Spoken into a room, both read as the entity ignoring someone, so the reply is
  treated as a draft: an incomplete one is merged with the completion the stream
  was cut from (`merge_completed_reply` splices on the overlap, so the seam is
  not heard twice), an unusable one is replaced with a fixed line.

## Prose tool-call envelope
- Owner: `shed-chat::document::settle_prose_tool_calls` (`agent.py:6149`), with
  `shed-chat::quality::strip_tool_call_envelope` and
  `shed-chat::quality::tool_calls_in_envelope` (`utils/reply.py`)
- Gates: whether a turn's tool calls survive, and whether the JSON envelope is
  spoken aloud.
- Fires: text that carries a `{tool_calls}…{/tool_calls}` envelope — the envelope
  is cut out of the reply and out of every bubble, a bubble left empty by the cut
  is dropped, the reply is rebuilt from the bubbles that survive, and the calls
  inside the envelope are recovered whenever the document carries none of its
  own.   Clears: text with no envelope, which is returned byte-identical.
- Config: none.
- Tests: `crates/shed-chat/tests/document.rs`
  (`fixture_settle_prose_tool_calls`, asserts the settled document) and
  `fixture_strip_tool_call_envelope` / `fixture_tool_calls_in_envelope` in
  `tests/quality.rs`.
- Note: the model wraps function calls in this envelope when it answers in prose
  instead of the reply document. Left alone, the listener hears a wall of JSON
  and no call is ever made. The guard runs inside the document parse, so a prose
  turn is settled exactly once, and it warns — naming the tools and quoting the
  reply — because a marker reaching the reply at all means a call was about to
  be lost.

Workspace gates already live (these guard the repo, not the runtime):

| Gate | Owner | Status |
|---|---|---|
| Frozen assets match `tests/assets.manifest.json` (both directions) | `shed-gates::assets` | **ported** |
| Crate layering + dev-only rules + map completeness | `shed-gates::layers` | **ported** |
| Only `shed-config` reads the environment | `shed-gates::env_read` | **ported** |
| `.env.example` documents exactly the read-set | `shed-gates::settings` | **ported** |
| Guard-catalogue honesty (stale locations, missing entries) | `shed-gates::guards` | pending (first guard) |
