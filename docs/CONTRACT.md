# The Frozen Contract

The browser client (`templates/index.html`, `static/app-helpers.js`,
`styles/`), the eight JS suites and the data files are copied byte-for-byte
from the Python app and **never edited** (decision D2, enforced by
`crates/shed-gates/src/assets.rs` against `tests/assets.manifest.json`). The
Rust server must satisfy exactly what they already speak. Line numbers cite
`agent.py` in the Python source at `D:\Entity-In-The-Shed` (readable until
retired at Stage 9). Any Rust change to this file that alters observable
behaviour is a client break; permitted divergences live in `PARITY.md`.

## 1. Cross-cutting

- **Error envelope** (`api_error_response` 987–999):
  `{"ok": false, "error": str, "code": str, "request_id": str}`, status as
  chosen by the handler, plus the `X-Request-ID` header on the response.
  `extra` dicts merge in.
- **Boundary** (`api_json_boundary` 1002–1012): any unhandled handler
  exception → `500` / `code: "internal_error"` / message
  `"internal server error"`, traceback logged with the request id. Rust: a
  `Result`-returning handler wrapper; **no panics reach the client**. Every
  `/api/*` route is boundary-wrapped **except** `/api/entity/stream` — the
  source explains why at 11477–11480: a JSON boundary cannot catch a
  generator error after the response object is returned. Rust keeps the same
  rule: SSE errors are handled inside the stream.
- **Request id** (1015–1027): assigned in `before_request` and echoed in
  `after_request` **only for paths starting `/api/`** — which is why `/health`
  carries no `X-Request-ID`. One axum middleware layer reproduces the scoping
  exactly.
- **Chat timing headers**: `X-Entity-In-The-Shed-Timing` (summary truncated
  to 1000 chars) and `X-Entity-In-The-Shed-Total-Ms` (one-decimal
  milliseconds).
- **Cache headers** on `/`, `/app.css`, `/app-helpers.js`, the webp:
  `Cache-Control: no-store, no-cache, max-age=0, must-revalidate`,
  `Pragma: no-cache`, `Expires: 0`.
- **`index.html` renders with exactly three substitutions** (Jinja `|tojson`):
  `chat_request_timeout_ms` (`LM_ENGINE_REPLY_TIMEOUT * 1000`, or `None`),
  `model_selection_requires_password`, `browser_tts_enabled`
  (`bool(TTS_BROWSER_FALLBACK)`) — render call at 8420–8425. A tiny
  substitution step replaces Jinja; no template-engine dependency.

## 2. `POST /api/chat` (9443–9510) and `GET /api/chat/jobs/<job_id>` (9389–9440)

- Body JSON; `deferred` parsed with a `parse_bool` equivalent, default
  `false`. Missing/empty `message` → `400` `code: "message_required"`.
- **Busy path:** when the single conversation slot cannot be taken
  non-blockingly → **HTTP 200**
  `{"ok": true, "reply": "The Entity is in consultation with another, please try later."}`.
  Not 429. (Text verified verbatim.)
- **Deferred path:** **HTTP 202**
  `{"ok": true, "pending": true, "job_id": <32-hex>, "status": "pending"}`;
  the job is registered
  `{status: "pending", created_at, expires_at: now + BROWSER_CHAT_JOB_TTL_SECONDS, sentences: []}`
  and runs on a worker; a spawn failure pops the job, releases the slot and
  surfaces the error.
- **Job status — three shapes, all load-bearing:**
  - unknown/expired → `404` `code: "chat_job_not_found"`;
  - pending → `{ok, pending: true, job_id, status, delivery_style, sentences[],
    sentence_count}`, where `delivery_style` is `""` until the worker resolves
    the turn's style (the page voices streamed sentences from it — empty is
    meaningful, not missing);
  - complete → the stored `response_payload` **with `sentences` re-attached**
    plus the timing headers; a stored failure replays the recorded
    `http_status` / `error_code` / message. The re-attachment exists because
    the turn publishes its closing sentence after the last poll that could
    have carried it (comment at 9400–9405); the client de-duplicates.
- **Single-slot rule** (`try_acquire_browser_chat_slot` +
  `AI_CONVERSATION_LOCK`): one conversation turn at a time, process-wide.
  Rust: one `TurnGate` owner with a test that reproduces the busy payload
  under two simultaneous requests. Slot release stamps `LAST_TURN_ENDED_AT` —
  the slow loop reads it.

## 3. Entity outbox + SSE

- `speak_to_client` (9600–9626) is **the single door**: every spoken line is
  queued (durable half) **and** pushed (live half); both halves are
  mandatory. Rust keeps one door.
- **Frame format** (`format_sse_frame` 9629–9640): optional `id: ` line, then
  `event: `, then one `data: ` line **per line of the JSON payload**
  (multi-line payloads split; `ensure_ascii=False`), terminated by a blank
  line.
- **Stream body** (`entity_stream_response` 11448–11470): first frame
  `event: state` with `{"client_id": ..., "ok": true}`, then catch-up
  `utterance` frames, then live frames with `": hb\n\n"` comment heartbeats,
  ending at `ENTITY_SSE_MAX_LIFETIME_SECONDS`. Headers:
  `Cache-Control: no-cache`, `X-Accel-Buffering: no`, and **`Connection` must
  not be set** — hop-by-hop headers belong to the HTTP layer (the source's
  comment at 11460–11465 explains the waitress `AssertionError` that taught
  this rule).
- **Refusals** (11482–11495): `404 entity_stream_disabled` (flag off),
  `429 entity_reconnect_too_fast`, `503 entity_stream_full`. Cursor from
  `?since=` or the leading integer of `Last-Event-ID` (ids are `<seq>-<ms>`).
- **Registry semantics** (9524–9597, all load-bearing): a reconnect for the
  same `client_id` **replaces and closes** the old subscriber (the cap bounds
  distinct clients, not connections); unregister is identity-checked so a
  departing old stream cannot evict its replacement; a full per-subscriber
  queue **drops the oldest** frame and keeps the newest (a wedged client must
  never stall the publisher); `close()` unblocks the parked generator.
- Outbox view (`outbox_message_payload` ~11053–11075): `seq`, `delivery_id`,
  `kind`, `trigger`, `text` (markup-normalised for reading), `text_markup`
  (raw, only when it differs), `character_slug`, `delivery_style`,
  `created_at`. `GET /api/entity/outbox` **drains**; a message stays
  deliverable until acked; `POST .../ack` is idempotent
  `{ok, delivery_id, changed}`.

## 4. `POST /api/speak` (8859–8941)

Request fields `text, voice, provider, character_id, preferred_language,
delivery_style, tone` — **`rate`/`pitch` are not read**; they come from the
delivery/tts maps. Success: audio file with its mimetype + `X-TTS-Voice`,
`X-TTS-Provider`, `X-TTS-Cache: hit|miss`, `Cache-Control: no-store`. Cache
hits come from the durable cache and are **not deleted**; misses are served
from a temp folder deleted shortly after serving. Failure:
`describe_tts_failure(...)` message with `500` / `code: "tts_failed"` — the
message text is contract because the client displays it.

## 5. Client invariants that pin the server

The deferred-chat flow (202 + job polling), sentence-by-sentence streaming
from `sentences[]`, the busy reply text, the outbox poll standing down while
the SSE stream is up, `client_id` minting/echoing, and the three template
substitutions are all load-bearing. Any "improvement" that changes one of
these is out of scope for the clone and goes to `TODO/Loose-Ends.md`.

## 6. Response-shape summary

| Route | Shape |
|---|---|
| `GET /health` | fixed JSON payload (captured byte-parity fixture, Stage 1) |
| errors | `{"ok": false, "error", "code", "request_id"}` + `X-Request-ID` (`/api/` only) |
| `POST /api/chat` | turn payload / busy 200 / deferred 202 `{ok, pending, job_id, status}` |
| `GET /api/chat/jobs/<id>` | 404 `chat_job_not_found` / pending shape / completed payload + re-attached `sentences` |
| `GET /api/entity/stream` | SSE: `state` first frame, `utterance` frames, `: hb` comments; never a `Connection` header |
| `GET /api/entity/outbox` | drained `{..., messages[]}`; ack `{ok, delivery_id, changed}` |
| `POST /api/speak` | audio bytes + `X-TTS-*` headers, or 500 `tts_failed` with human message |
