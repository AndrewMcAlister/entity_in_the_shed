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
| Flask/Jinja rendering | the frozen page needs exactly 3 substitutions | `agent.py:8420–8425` |

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
| A model's metadata record carries only the keys the app reads, where the source copies the payload's whole dict | principle 3/12: a typed record; no path in the source produces a key outside that set |
| A model size a payload spells as non-numeric reads as "no size" | the source let the conversion raise `ValueError` out of `discover_lm_studio_models` and take the request with it; model discovery is best-effort, and one malformed field must not lose the whole catalog |
| Conversation lock + chat slot + active-job counter + `LAST_TURN_ENDED_AT` become one `TurnGate` owner | principles 6/17 |
| Windows service name `entity_in_the_shed` and default storage root `C:\Temp\entity_in_the_shed` differ from the Python app's `entity-in-the-shed` / `C:\Temp\entity-in-the-shed` | decision D9: coexistence — two live services must not share a registration, a port, or mutable state on disk |
| Env keys: every key UPPERCASE; percentage keys end `_PERCENT` with a double-quoted value ending in `%` (e.g. `ENTITY_MARKET_MOVE_PERCENT="2.0%"`) parsed to `f64` fractions once at startup; `APP_HOST`/`APP_PORT` only (the source's `HOST`/`PORT`/`FLASK_HOST`/`FLASK_PORT` fallback chain is not read) | principle 24; the source itself violates it (`lm_engine_COMPLETION_NUM_PREDICT` @403) |

### What must **not** change

`CONTRACT.md` in full: the queue-**and**-push delivery door, the single-turn
gate, the 202 + job-polling flow, the busy reply text, the `sentences[]`
shape and re-attachment, the `text` vs `text_markup` split, the SSE registry
semantics, and the frozen client files.

## Stage status

| Stage | Scope | Parity status |
|---|---|---|
| 0 | workspace skeleton, frozen assets + manifest, gates | asset gate green; JS suites green unchanged |
| 1 | config, logging, `/health`, smoke boot | pending — `/health` byte-parity vs captured fixture |
| 2 | models + LLM transport | pending |
| 3 | prompts, characters, pages, catalogue routes, envelope/request-id middleware | pending |
| 4 | conversation + chat (turns, jobs, `TurnGate`) | pending |
| 5 | tools + MCP | pending |
| 6 | TTS (Edge spike, SAPI, cache, speak routes) | pending |
| 7 | entity + mind (outbox, SSE, slow loop) | pending |
| 8 | ops (restart, unload, keyring, service `entity_in_the_shed`) | pending |
| 9 | side-by-side sweep, unknown-key warning, plan deletion | pending |
