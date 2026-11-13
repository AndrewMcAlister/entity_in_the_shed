# Parity

The clone reproduces the Python app's observable behaviour except where this
file says otherwise. Anything not listed here is a parity bug. Each stage
ticks its rows; Stage 9's side-by-side sweep (every route hit on both apps,
JSON diffed) must come back empty except these rows.

## Divergences

### Surfaces the clone does not carry

| Surface | Why | Evidence |
|---|---|---|
| `GET /_routes`, `GET /api/debug-voices`, `GET /api/debug-models`, `POST /api/set-model` | Flask introspection artifacts / duplicates / legacy mode-switch leftover | 0 references in `index.html` + `app-helpers.js`, counted before Stage 0 |
| `MCTRADER_BASE_URL`, `MCTRADER_API_TOKEN` | read once, used nowhere else | re-grep at Stage 5 before dropping |
| `initialize_mctrader_credentials` | body is `pass` (`agent.py:11519`) | read |
| `powershell_command` + `fetch_AI_MODEL_names` | PowerShell model discovery; `discover_lm_studio_models_from_disk` is the native path the app depends on | `agent.py:1677`/`1722` |
| `discover_lm_studio_models`'s `model_names` branch | dead code: `model_names = []` is its only assignment, so the `if model_names: break` above it can never fire | `agent.py:1543`/`1574`/`1596-1600`, read at Stage 2 |
| `last_model_index.txt` in the repo root | moved under `STORAGE_ROOT` | principle 19 |
| IMAP email tools (`agent.py:5076–5225`) + the five `EMAIL_*` keys | separate concern; keys read by the code and documented nowhere in the source | env sweep before Stage 0 |
| `USE_WAITRESS` / `WAITRESS_THREADS` / `WAITRESS_CHANNEL_TIMEOUT` as a toggle | one server: tokio + axum sized like the waitress branch (32 workers, 600 s channel timeout) | the source's own `.env.example` ships `USE_WAITRESS=0`; the SSE stream needs the waitress-sized shape regardless |
| `/api/voices`'s `?refresh=1` re-fetching live Edge/Windows voices | the voice list is the frozen `voices.json` snapshot until Stage 6 installs the live lookups, so there is nothing to re-fetch; the parameter is accepted and ignored, because the client sends it and rejecting a query the server ignores would be a new failure mode | `agent.py:8499`, `picker.rs` |
| Flask/Jinja rendering | the frozen page needs exactly 3 substitutions | `agent.py:8420–8425` |
| `/api/voices` answers the 323 voices of the frozen `voices.json` where the running app answers 333 | the live list is Edge's 323 **plus** the 10 Windows desktop voices `list_windows_voices` adds even when `TTS_PROVIDER=edge` (`agent.py:3232`); and the snapshot is a captured *response body*, so it lost the order that settles ties — within equal `(matches, score)` the live answer follows the Edge service's raw list order, which is neither alphabetical nor the snapshot's. One row also went stale: `fil-PH-BlessicaNeural` carries the Filipino male voice's friendly name and `Male` gender because Edge renamed that pair after the capture. Measured against the running app: the envelope and every field of the other 322 shared voices are byte-identical, so all three differences are the frozen catalogue and Stage 6's live fetch removes them | measured 2026-11-10, `picker.rs`, `catalog.rs` |

`POST /api/test-speak` **is** ported despite 0 client references: it is the
voice-audition surface `MARKUP_TESTING.md` describes and 5 Python test
references pin its behaviour. `/api/restart` is client-referenced;
`/api/keyring` and `/api/unload-models` are documented operator surfaces with
no Python tests — all three port at Stage 8 with **new** Rust tests.

### Structural changes (same behaviour, different internals)

| Change | Reason |
|---|---|
| One `AppConfig` replaces 148 scattered env reads and their derived constants | principle 19 |
| `run_agent` (~332 lines) and `call_lm_engine` (~324 lines) decompose into plan → call → parse → recover → persist | principles 2/5 |
| The retry ladder (fallback model, temperature, tools, json-mode, reasoning-effort, max-completion-tokens) becomes one ordered `RetryStrategy` chain | principle 11 |
| The ~20 slow-loop vetoes become named guard functions with catalogue entries | principle 23 |
| `synthesize_speech` (~88 lines) splits into voice choice → provider call → audio validation → cache admission, with one delete path | principles 2/17 |
| Reply-document parsing (`ast.literal_eval` acceptance) becomes strict JSON with an explicit, catalogued repair step | principles 3/18 |
| `models_cache.json` is written with LF line endings | the source opens the file in text mode, so the same cache is CRLF on Windows; the clone writes one shape on every platform, and only the clone ever reads it |
| Every JSON document the clone stores is LF, where the source's are CRLF on Windows (`atomic_write_json`, `entity_store.py:62`, opens in text mode) | same reason and same scope as the cache row above: conversation records, mind-state and client documents are read back by whichever app wrote them — decision D9 gives the clone its own `STORAGE_ROOT`, so no file crosses the two apps — and JSON ignores line endings. The `.bak` copy is a byte copy of whatever was there, so a pre-clone backup keeps its CRLF and is still read correctly. One smaller difference rides along: the temp file's random middle is a uuid4 hex where `tempfile.mkstemp` used eight characters, so a crash mid-write leaves a longer name in the directory — the file is renamed or deleted before every successful call returns, and its name is never a wire value (`json_doc.rs`) |
| A model's metadata record carries only the keys the app reads, where the source copies the payload's whole dict | principle 3/12: a typed record; no path in the source produces a key outside that set |
| A model size a payload spells as non-numeric reads as "no size" | the source let the conversion raise `ValueError` out of `discover_lm_studio_models` and take the request with it; model discovery is best-effort, and one malformed field must not lose the whole catalog |
| Conversation lock + chat slot + active-job counter + `LAST_TURN_ENDED_AT` become one `TurnGate` owner | principles 6/17 |
| Windows service name `entity_in_the_shed` and default storage root `C:\Temp\entity_in_the_shed` differ from the Python app's `entity-in-the-shed` / `C:\Temp\entity-in-the-shed` | decision D9: coexistence — two live services must not share a registration, a port, or mutable state on disk |
| A port another process already holds stops the clone at bind (`cannot bind 127.0.0.1:5056: … os error 10048`, exit 1); Werkzeug binds it a second time and says nothing | `werkzeug.serving` sets `allow_reuse_address=True`, and on Windows that lets a second listener join a live port — measured during the Stage-3 gate, when the reference app came up on 5005 while the service held it: both were live and only one was reachable, which is the failure D9 exists to prevent. The loud refusal is the behaviour the installer needs, and `--smoke` cannot collide because it binds an ephemeral port (`main.rs`) |
| Env keys: every key UPPERCASE; percentage keys end `_PERCENT` with a double-quoted value ending in `%` (e.g. `ENTITY_MARKET_MOVE_PERCENT="2.0%"`) parsed to `f64` fractions once at startup; `APP_HOST`/`APP_PORT` only (the source's `HOST`/`PORT`/`FLASK_HOST`/`FLASK_PORT` fallback chain is not read) | principle 24; the source itself violates it (`lm_engine_COMPLETION_NUM_PREDICT` @403) |
| The engine's socket timeout (`timeout=(idle, idle)`, `agent.py:1176`) becomes an idle window applied to each wait separately — the wait for a response head, then every read of the streamed body — rather than to reqwest's `timeout`, which is one budget for the whole exchange | a stream that speaks slowly for longer than one window is legitimate and must not be cut off; the source's tuple restarts on every chunk, and the total guard is the caller's deadline (`agent.py:6611`), which the production caller always sets |
| A streamed body that breaks mid-way is retried only for the failure `requests` would retry: an expired idle window. A connection that dies inside a body already begun re-raises at any attempt count | the source's retryable pair (`agent.py:1032`) is `(Timeout, ConnectionError)`, and a body frame that ended early is `ChunkedEncodingError`, which is neither — so reqwest's `Kind::Body`/`Kind::Decode` shape of that break is deliberately outside the clone's streaming classifier |
| The three stream failures are types — `LmStreamUnavailable` (nothing spoken yet, the caller may fall back), `StreamTimeout`, `StreamFailure::{Timeout, Transport}` — whose `Display` keeps the source's wording wherever a user reads it | principles 3/18: the caller decides on the *class* (`agent.py:7016`), while `str(exc)` reaches `api_error_response` verbatim; a prefix on the message would change what the log and the client see |
| An expired idle window reads `the engine sent no bytes for 150ms` | the source's words are urllib3's (`Read timed out. (read timeout=150)`), and this text reaches only a log line; naming the window is what makes it readable next to the reply budget it expired |
| The progress sink is `Fn(&str)`, so the source's `try/except` around `on_delta` (`agent.py:1143-1148`) has nothing to catch | a Rust callback cannot fail without saying so in its signature, and the sink's only job is to hand on a `&str`; a sink that could fail would catch its own error where the source caught it |
| `index.html` is substituted once and served from the state's `OnceLock`; the source runs Jinja per request | the three holes are filled from settings that cannot change while the process runs, so the bytes are identical either way, and re-rendering per request would re-read and rescan the page for every keystroke. The substitution engine copies Jinja's two text-mode habits, because they are 4531 bytes of the page: the template is read through universal newlines (the CRLF file on disk reaches the browser as LF) and one trailing newline is dropped (`keep_trailing_newline=False`) — measured, and pinned by `frontend_cache_headers.rs` (`page.rs`) |
| The `except` arms of the six Stage-3 views (`@api_json_boundary`, `agent.py:1002-1012`) have no counterpart | every helper those routes call is total — each logs and returns the empty value where the Python original raises — so the arms have no trigger in either app; the envelope (`api_error_response`, `agent.py:987`) joins the first route that can reach it, Stage 4's `chat_job_not_found` (`agent.py:9409`). `set-last-model-index` is the Stage-3 route that does use it: its `request.get_json()` raises on a missing, wrongly-typed or malformed body in both apps, and both answer the 500 envelope |
| `/api/refresh-models` discovers into a fresh `ModelCatalog` and swaps it under a `Mutex`; the source rewrites the `KNOWN_MODELS`/`MODEL_METADATA` module globals in place (`agent.py:1256`, `11595`) | the lock covers only a synchronous read or the swap, never an await, so no response can observe a half-written catalog and the routes need no lock of their own |
| The catalog warmer runs on `tokio::task::spawn_blocking`; the source starts it on a daemon thread before the banner (`agent.py:2704`) | the work is synchronous file I/O, and a warmer that blocked a request thread would make the warm-up slower than the first request it exists to speed up |
| The four no-store routes send no `ETag`/`Last-Modified` and never answer 304 | Werkzeug builds the ETag from the file's mtime as a float two runtimes cannot agree on byte-for-byte, and every response already says `no-store`, so a conformant client never revalidates; measured against the running app (`asset.rs`) |
| A query parameter repeated in one URL takes its **last** value where Flask's `.get()` takes its first | `axum::Query` deserialises over a `MultiMap`; the frozen client sends each name once, so there is no defined behaviour to match (`api/mod.rs`) |

### Stored conversation stamps (Stage 4)

| Difference | Why | Evidence |
|---|---|---|
| A time-only stamp (`"12:00:00"`) is unreadable here; Python 3.11+ reads it as today at noon | a bare time names no day, so it cannot order a transcript, and every stamp this app stores is an `isoformat()` date-plus-time | `timestamp.rs`, `an_absent_or_unreadable_stamp_is_none` |
| A short offset (`+02`, `+0230`) parses in Python and not through RFC 3339 | the app writes only the full `±HH:MM`; `DateTime::parse_from_rfc3339` refuses the short forms | `timestamp.rs` |
| A fraction longer than nine digits is refused here and accepted by Python | `isoformat()` emits at most six digits, and chrono's `%.f` caps at nine | `timestamp.rs` |
| An aware stamp against a naive cutoff answers "recent" (and no gap); Python raises `TypeError` | Rust must not panic where Python raises, and the pair is unreachable from the app: every cutoff comes from `datetime.now(timezone.utc)` and every stored stamp carries the offset that clock wrote. "Recent" is the same answer an unreadable stamp gets | `timestamp.rs`, `an_aware_stamp_against_a_naive_cutoff_is_recent_rather_than_a_crash` |
| A row that is not a JSON object fails when the record is loaded (slice 4d) rather than being skipped per loop | `unprofiled_messages` takes `&[ConversationMessage]`, so the row shape settles at parse time; the source tolerates a bad row because it reads raw JSON | `freshness.rs` |

Not a divergence, and pinned by a test: where the source does `parsed.replace(tzinfo=cutoff.tzinfo)`, the clone **attaches** the aware side's offset to the naive wall time instead of converting the instant, so the two answers agree even far from UTC (`timestamp.rs`, `a_naive_stamp_takes_the_aware_cutoffs_offset_rather_than_converting`).
### What must **not** change

`CONTRACT.md` in full: the queue-**and**-push delivery door, the single-turn
gate, the 202 + job-polling flow, the busy reply text, the `sentences[]`
shape and re-attachment, the `text` vs `text_markup` split, the SSE registry
semantics, and the frozen client files.

## Stage status

| Stage | Scope | Parity status |
|---|---|---|
| 0 | workspace skeleton, frozen assets + manifest, gates | asset gate green; JS suites green unchanged |
| 1 | config, logging, `/health`, smoke boot | green — `/health` byte-parity against the live capture; `--smoke` exits 0 |
| 2 | models + LLM transport | green — divergences recorded above (idle-window shape, streaming retry set, LF cache file, typed metadata record); fake-engine suites green |
| 3 | prompts, characters, pages, catalogue routes, envelope/request-id middleware | green — page and assets byte-identical through the frozen-asset gate; header and payload shapes measured against the running app; Stage-3 divergences recorded above |
| 4 | conversation + chat (turns, jobs, `TurnGate`) | pending |
| 5 | tools + MCP | pending |
| 6 | TTS (Edge spike, SAPI, cache, speak routes) | pending |
| 7 | entity + mind (outbox, SSE, slow loop) | pending |
| 8 | ops (restart, unload, keyring, service `entity_in_the_shed`) | pending |
| 9 | side-by-side sweep, unknown-key warning, plan deletion | pending |
