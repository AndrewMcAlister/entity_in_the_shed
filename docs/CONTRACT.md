# The frozen contract

What `templates/index.html` and `static/app-helpers.js` already expect, verified against the Python
source. The Rust server reproduces it exactly; a change here is a client break, so this file is the
spec, not a summary. Python line numbers point at `D:\Entity-In-The-Shed\agent.py` for traceability.

## 1. Cross-cutting

- **`X-Request-ID`** is assigned before every request (`1015–1020`), echoed after every response
  (`1023–1027`) and repeated inside every error body. Rust: one axum middleware layer on every route.
- **Error envelope** (`api_error_response`, `987–999`):

  ```json
  {"ok": false, "error": "<message>", "code": "<machine code>", "request_id": "<id>"}
  ```

  with the status the handler chose. `api_json_boundary` (`1002–1012`) turns any unhandled handler
  exception into `500` / `code: "internal_error"` after logging the traceback with the request id.
  Rust: a `Result`-returning handler wrapper — **no panic may reach the client**.
- Every `/api/*` handler is boundary-wrapped **except** `/api/entity/stream` and `/_routes`
  (`11474–11481`): a JSON boundary cannot catch a generator error after the response object has been
  returned. The same rule holds in Rust — SSE errors are handled inside the stream.
- **Static and page responses** carry `Cache-Control: no-store, no-cache, max-age=0, must-revalidate`,
  `Pragma: no-cache`, `Expires: 0` (`index` 8427–8429, `app.css` 8440–8442, and the same on
  `app-helpers.js` and the webp).
- **`index.html` is rendered with exactly three substitutions** and nothing else:
  `chat_request_timeout_ms` (`LM_ENGINE_REPLY_TIMEOUT * 1000`, or `None`),
  `model_selection_requires_password` (`MODEL_SELECTION_REQUIRES_PASSWORD`),
  `browser_tts_enabled` (`bool(TTS_BROWSER_FALLBACK)`). The template keeps its Jinja-looking
  `{{ name|tojson }}` placeholders; the Rust side substitutes them and does not evaluate anything else.
- **Chat timing headers**: `X-Entity-In-The-Shed-Timing` (truncated to 1000 chars) and
  `X-Entity-In-The-Shed-Total-Ms` (`f"{total_ms:.1f}"` formatting, one decimal).

## 2. `POST /api/chat` (`9443–9510`)

- Body: JSON. `deferred` is parsed as a bool with default `false`. A missing or empty `message` is
  `400` / `code: "message_required"`.
- **Busy path** — when the conversation lock cannot be taken without blocking: **HTTP 200** with

  ```json
  {"ok": true, "reply": "The Entity is in consultation with another, please try later."}
  ```

  Not 429. The client displays this text, so the wording is part of the contract.
- **Deferred path** — **HTTP 202** `{"ok": true, "pending": true, "job_id": "<32 hex>",
  "status": "pending"}`. The job is registered with `expires_at = now + BROWSER_CHAT_JOB_TTL_SECONDS`
  and `sentences: []`, and runs on a worker. Releasing the slot stamps `LAST_TURN_ENDED_AT`
  (`9063–9065`), which the slow loop reads.
- **`GET /api/chat/jobs/<job_id>`** (`9389–9440`) has three shapes and all three are the contract:
  - unknown or expired id → `404` / `code: "chat_job_not_found"`;
  - `status == "pending"` → `{ok, pending: true, job_id, status, delivery_style, sentences[],
    sentence_count}`, where `delivery_style` is `""` until the worker has resolved the turn's style.
    An empty value is meaningful, not missing: the page voices streamed sentences using it;
  - `status == "complete"` → the stored `response_payload` **with `sentences` re-attached**, plus the
    chat timing headers. A stored failure replays the recorded `http_status` / `error_code` / message.

  `sentences` is re-attached on purpose: the turn publishes its closing sentence after the last poll
  that could have carried it, so the completed payload is the only remaining place the final sentence
  can arrive from. The source's own comment says dropping it yields "a reply that is printed in full
  and spoken short of its ending". The client de-duplicates, and the Rust port keeps both halves.
- **Single slot**: one conversation turn at a time, process-wide (`ACTIVE_CHAT_REQUESTS` plus the
  conversation lock). This is the app's concurrency contract, not an implementation detail.

## 3. `POST /api/speak` (`8861–8941`)

- Request fields: `text`, `voice`, `provider`, `character_id`, `preferred_language`,
  `delivery_style`, `tone`. **`rate` and `pitch` are ignored** — they come from the delivery style and
  then the tone (`get_delivery_tts_settings` then `apply_tone_tts_settings`).
- Success: the audio bytes with their mimetype, plus `X-TTS-Voice`, `X-TTS-Provider`,
  `X-TTS-Cache: hit|miss`, `Cache-Control: no-store`.
- Cache hits are served from the durable cache and **not deleted**. Misses go to a temp folder that is
  deleted (file, then the empty parent directory) about a second after serving.
- Failure: `500` / `code: "tts_failed"` with the message from `describe_tts_failure(...)`. The message
  text is part of the contract because the client displays it.

## 4. Entity outbox and SSE

- `outbox_message_payload` (`11053–11075`) is the only view of a queued message: `seq`, `delivery_id`,
  `kind`, `trigger`, `text` (markup-normalised for reading), `text_markup` (the raw text, only when it
  differs), `character_slug`, `delivery_style`, `created_at`. Provenance stays server-side.
- `GET /api/entity/outbox?client_id=&limit=` → `{ok, client_id, messages[], count}`. It **drains**, and
  a message stays deliverable until it is acked.
- `POST /api/entity/outbox/<delivery_id>/ack` → `{ok, delivery_id, changed}`. Idempotent.
- `GET /api/entity/stream` (`11474–11499`): refusals are `404 entity_stream_disabled`,
  `429 entity_reconnect_too_fast`, `503 entity_stream_full`. The cursor comes from `?since=` or the
  leading integer of `Last-Event-ID`, because ids are `<seq>-<ms>`.
- **Frame format** (`format_sse_frame` `9629–9640`): optional `id:`, then `event:`, then one `data:`
  line **per line of the JSON payload** — a multi-line payload is split into several `data:` lines —
  terminated by a blank line.
- **Stream body** (`entity_stream_response` `11448–11470`): first frame `event: state` with
  `{"client_id": ..., "ok": true}`, then catch-up `utterance` frames, then live frames, with
  `": hb\n\n"` comment heartbeats every `ENTITY_SSE_HEARTBEAT_SECONDS`, ending at
  `ENTITY_SSE_MAX_LIFETIME_SECONDS`. Headers: `Cache-Control: no-cache`, `X-Accel-Buffering: no`, and
  **`Connection` must never be set** — the source explains that its WSGI server rejects that header
  from an app (`11458–11463`), and the Rust server keeps the rule for the same reason.
- `speak_to_client` (`9600–9626`) is the single door for a spoken line: it queues **and** pushes. Both
  halves are mandatory, and the Rust port keeps one door for the same reason.

## 5. Client behaviour that pins the server

`index.html` and `app-helpers.js` are not edited, so all of these are load-bearing:

| Client behaviour | Where |
|---|---|
| Deferred chat: 202 then polling | `index.html:682` — `chatJobPollIntervalMs = 250` |
| Streams sentences to speech from `sentences[]` | `index.html` + `app-helpers.js` |
| Displays the busy reply text verbatim | §2 |
| Outbox polling stands down while the SSE stream is up | `index.html:630–636` |
| Keys the proactive stream off `proactive_enabled` (a boolean in the state payload) | `index.html:630–632` |
| Mints and echoes `client_id` | `app-helpers.js` |
| Reads the three template substitutions | `index.html:132`, `678`, `679` |

Any Rust-side improvement that changes one of these is out of scope for the clone and goes to
`TODO/Loose-Ends.md`.
