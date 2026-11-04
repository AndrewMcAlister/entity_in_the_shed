# Parity checklist — Python app → Rust clone

Companion to `TODO/PLAN-Rust-Clone.md`. One row per surface of the Python app. Tick **Ported** only
when its tests pass. This is the single parity table: principle 20 means an unticked row is unfinished
work, and a row marked *absent* must say why.

Legend — **Client**: the browser client calls it (it is load-bearing for the frozen frontend).
**Ops**: only tests/operators call it. **None**: nothing calls it.

## 1. HTTP routes (35)

| # | Route | Client | Stage | Decision | Ported |
|---|---|---|---|---|---|
| 1 | `GET /` | Client | 3 | port - serves `templates/index.html` with 3 substitutions | ☐ |
| 2 | `GET /app.css` | Client | 3 | port — byte-identical, no-store headers | ☐ |
| 3 | `GET /app-helpers.js` | Client | 3 | port — byte-identical, no-store headers | ☐ |
| 4 | `GET /styles/information_info_1565.webp` | Client | 3 | port | ☐ |
| 5 | `GET /api/countries` | Client | 3 | port | ☐ |
| 6 | `GET /api/characters?country=&language=` | Client | 3 | port — roster tiering, voice filter, `details` | ☐ |
| 7 | `GET /api/voices` | Ops | 3 | port — the client falls back to browser voices, the catalogue is still needed | ☐ |
| 8 | `GET /api/models` | Client | 3 | port | ☐ |
| 9 | `GET /api/get-last-model-index` | Client | 3 | port | ☐ |
| 10 | `POST /api/set-last-model-index` | Client | 3 | port | ☐ |
| 11 | `POST /api/chat` | Client | 4 | port — always `deferred: true` from this client → 202 + job id | ☐ |
| 12 | `GET /api/chat/jobs/<job_id>` | Client | 4 | port — 250 ms polling (`index.html:682`); three payload shapes: `404 chat_job_not_found`, pending `{ok,pending,job_id,status,delivery_style,sentences,sentence_count}`, complete = stored payload **with `sentences` re-attached** + timing headers; a stored failure replays its recorded status/code | ☐ |
| 13 | `POST /api/speak` | Client | 6 | port — server derives rate/pitch from `delivery_style`+`tone` | ☐ |
| 14 | `GET /api/reply-laugh` | Client | 6 | port — `reply_laugh_url` is the literal `/api/reply-laugh` or `""` | ☐ |
| 15 | `GET /api/reply-fart` | Client | 6 | port — carries `X-Playback-Rate` | ☐ |
| 16 | `POST /api/mcp/auth` | Client | 5 | port | ☐ |
| 17 | `GET /api/entity/state?client_id=` | Client | 7 | port — mints/echoes `client_id` | ☐ |
| 18 | `GET /api/entity/outbox?client_id=` | Client | 7 | port — drains | ☐ |
| 19 | `POST /api/entity/outbox/<delivery_id>/ack` | Client | 7 | port — idempotent, `changed` flag | ☐ |
| 20 | `POST /api/entity/proactive` | Client | 7 | port — the chatterbox toggle | ☐ |
| 21 | `GET /api/entity/stream` | Client | 7 | port — SSE, `state` then `utterance`, heartbeats | ☐ |
| 22 | `POST /api/entity/events` | Ops | 7 | port — entity ingestion | ☐ |
| 23 | `POST /api/entity/say` | Ops | 7 | port | ☐ |
| 24 | `GET /health` | Ops | 1 | port — byte-identical against the captured 289-byte fixture (key order, `Content-Type`, `Content-Length`, one trailing `\n`); `mctrader_mcp.tool_count` and `tools` stay `0` and `[]` until Stage 5 (§10) | ☑ |
| 25 | `POST /api/refresh-models` | Ops | 3 | port | ☐ |
| 26 | `POST /api/unload-models` | Ops | 8 | port — router admin | ☐ |
| 27 | `POST /api/restart` | Client | 8 | port — the client's restart button reads `error`/`message` | ☐ |
| 28 | `GET /api/keyring` | Ops | 8 | port, re-shaped — names the process account | ☐ |
| 29 | `GET /api/mcp/tools` | Ops | 5 | port | ☐ |
| 30 | `POST /api/mcp/tools/refresh` | Ops | 5 | port | ☐ |
| 31 | `GET /api/debug-voices` | None | — | **absent** — duplicates `/api/voices`, nothing calls it | n/a |
| 32 | `GET /api/debug-models` | None | — | **absent** — duplicates `/api/models` | n/a |
| 33 | `GET /_routes` | None | — | **absent** — a Flask introspection artifact | n/a |
| 34 | `POST /api/set-model` | None | — | **absent** — legacy mode-switch leftover; the launchers it served were removed | n/a |
| 35 | `POST /api/test-speak` | None | 6 | **port** — the voice audition for `MARKUP_TESTING.md`: raw text with **no** markup normalisation, explicit `rate` (float, default 1.0) and `pitch` (float, default 72) via `parse_tts_test_tuning` (`agent.py:8591`), `X-TTS-Provider`, codes `no_text` / `no_voice` / `unsupported_provider` / `synthesis_failed` / `test_speak_error`. It answered 500 to every request for its whole life until `tests/test_test_speak_route.py` pinned it; that suite is ported unchanged | ☐ |

Middleware: `before_request` request-id assignment (`agent.py:1014–1020`), `after_request`
`X-Request-ID` echo (`1022–1027`) — **Stage 3**, not Stage 1. Both hooks are `/api/`-only in the source:
`assign_request_id` returns unless `request.path.startswith("/api/")` and `attach_request_id_header` sets
the header for the same paths, which is why the captured `/health` response carries no `X-Request-ID`.
Stage 1's only route is `/health`, so middleware for `/api/*` would guard nothing until Stage 3 serves
`/api/countries`. `@api_json_boundary` (`1002–1012`) wraps every `/api/*` handler **except**
`/api/entity/stream` and `/_routes`, which the source explains at `11474–11481`; the clone keeps that
exception, and it lands with the same stage. Stage numbers below refer to §8 of `PLAN-Rust-Clone.md`.

## 2. Background loops

| Loop | Stage | Ported |
|---|---|---|
| Conversation housekeeper (idle trim + profile update) | 4 | ☐ |
| Cleanup scheduler (speech 7 d, conversations 365 d, logs 7 d, stale TTS temp) | 8 | ☐ |
| Character catalogue warmer | 3 | ☐ |
| Mind slow loop (jittered tick, dry-run default true) | 7 | ☐ |
| Conversation profile workers | 4 | ☐ |
| Per-request browser-chat job worker | 4 | ☐ |

Four of these are **off by default in the source**, in the code and in `.env.example`, and the clone keeps
those defaults: `ENTITY_PROACTIVE_ENABLED=false` (`agent.py:742`), `ENTITY_SLOW_LOOP_ENABLED=false` (782),
`ENTITY_SAY_ENABLED=false` (746), `ENTITY_SSE_ENABLED=false` (760), with `ENTITY_DRY_RUN=true` (785). They
are one opt-in chain. `/api/entity/state` answers it per client: `proactive_enabled` is *this client's*
answer and is the only field the page turns its stream and poll on (`index.html:631–636`), while
`proactive_available`, `sse_enabled` and `say_enabled` report the server-side master switches
(`agent.py:11268–11271`) — `sse_enabled` is what makes `/api/entity/stream` refuse with
`404 entity_stream_disabled`, and `say_enabled` gates `/api/entity/say`. Stage 7 is gated twice: once with
the chain switched on, once with the defaults to prove the silent state is silent.

## 3. Local tools

| Tool | Stage | Ported |
|---|---|---|
| `current_datetime` | 5 | ☐ |
| `search_web` (DuckDuckGo, forced-search markers) | 5 | ☐ |
| `fetch_webpage` (localhost/private blocked) | 5 | ☐ |
| `create_spreadsheet` (CSV under `STORAGE_ROOT`) | 5 | ☐ |
| `mctrader_status` / `start_mctrader` / `stop_mctrader` | 5 | ☐ |
| `list_recent_emails` / `read_email` | — | **absent** — decided: IMAP is a separate concern with undocumented credentials (`EMAIL_USERNAME` / `EMAIL_PASSWORD` / `EMAIL_IMAP_HOST` / `EMAIL_IMAP_PORT` / `EMAIL_FOLDER` are read but appear in no `.env.example`). Recorded here, restorable from `TODO/Loose-Ends.md` | n/a |
| McTrader MCP tools (42 discovered, name-based auth gating) | 5 | ☐ |

## 4. Model response contract

| Item | Stage | Ported |
|---|---|---|
| `{"reply", "tool_calls"}` document, `replies`/`reply_segments` variants | 4 | ☐ |
| `ast.literal_eval` acceptance (only when a reply key appears) → a strict JSON-with-repair path | 4 | ☐ |
| Tone tags `{name}…{/name}` rules, sounds `mmm`/`ahhh` (`utils/text.py`) | 1 | ☐ |
| Prose `{tool_calls}` envelope salvage (`utils/reply.py`) | 4 | ☐ |
| `looks_incomplete` / `looks_unusable_reply` / `looks_out_of_character_reply` / `looks_like_recovery_meta_reply` / `looks_like_generic_handoff_reply` / `looks_like_action_teaser_reply` / `looks_like_task_stub_reply` (`utils/reply.py`) | 4 | ☐ |
| `JsonReplyStreamScanner` / `SentenceSplitter` / `ReplyStreamPublisher` | 4 | ☐ |
| Recovery ladder bounds (1 follow-up, ≤ 2 continuations, 1 usable retry) | 4 | ☐ |
| `merge_completed_reply` overlap merge (`utils/reply.py:170`) | 4 | ☐ |

## 5. Storage layout under `STORAGE_ROOT`

| Path | Stage | Ported |
|---|---|---|
| `STORAGE_ROOT` itself, resolved and created at startup | 1 | ☑ |
| `Conversations/<client-id or ip>.json` | 4 | ☐ |
| `Conversations/<slug>/character-outline.json` + per-character profile folders | 3 | ☐ |
| `Mind/clients/` | 7 | ☐ |
| `Mind/users/` | 7 | ☐ |
| `Mind/outbox/` | 7 | ☐ |
| `Mind/<slug>/state.json`, `goals.json`, `events.jsonl`, thoughts | 7 | ☐ |
| `Logs/` — the directory and the daily log file | 8 | ☐ |
| `tts-cache/` | 6 | ☐ |
| `models_cache.json`, `model_local_map.json` | 2 | ☐ |
| `.bak` and `.corrupt-<stamp>` quarantine side files | 4 | ☐ |
| last-model-index file (source keeps it in the repo root; the clone keeps it under `STORAGE_ROOT`) | 3 | ☐ |

`STORAGE_ROOT` default changes from `C:\Temp\entity-in-the-shed` to `%LOCALAPPDATA%\entity-in-the-shed`
(the source README itself says `C:\Temp` destroys `Mind`), falling back to `$HOME/entity-in-the-shed` and
then to a bare `entity-in-the-shed` when neither is set. Recorded as a fixed defect, not a surprise.

**Stage 1 creates the root and nothing inside it.** The source creates `Logs/` at import time
(`agent.py:311–312`) although the daily log file is written by the service host, which is Stage 8; the
clone creates a directory only when a stage has code that puts a file in it. Stage 1's log lines go to
stderr, which is where the source's own handler writes them (`configure_logging`, `11524–11550`), and the
service host is what redirects that stderr into `Logs/`.

## 6. Config keys

**113 keys** in the source `.env.example`, but the source **reads 150 distinct env names** across
`agent.py` + `utils/` + `scripts/` — so 36 names the code honours are undocumented (`EMAIL_*`,
`ENTITY_MAX_UTTERANCES_PER_{HOUR,DAY}`, `ENTITY_MIN_UTTERANCE_GAP_SECONDS`,
`ENTITY_REPEAT_GUARD_LINES`, `CONVERSATION_FILE_RETENTION_DAYS`, `GENERATED_FILE_RETENTION_DAYS`,
`SERVICE_LOG_RETENTION_DAYS`, `CLEANUP_INTERVAL_SECONDS`, `LM_ENGINE_REPLY_TIMEOUT`,
`LM_ENGINE_TRANSIENT_RETRIES`, `LM_ENGINE_RETRY_{BASE,JITTER}_MS`, `LM_ENGINE_MODEL_ADMIN_TIMEOUT`,
`LM_ENGINE_RESIDENCY_PROBE_TIMEOUT`, `MCTRADER_DEFAULT_{BROKER,EXCHANGE,WATCHLIST}_ID`,
`MCTRADER_MCP_{TIMEOUT,CACHE_TTL}`, `MCTRADER_CONTROL_CWD`, `BROWSER_CHAT_JOB_{TTL_SECONDS,MAX_SENTENCES}`,
`TTS_TIMEOUT`, `TTS_DEFAULT_RATE`, `WEB_SEARCH_ENABLED`, `MODEL_FALLBACK_TIMEOUT_SECONDS`,
`AI_MODEL_DISCOVERY_TIMEOUT`, `AI_MODEL_LOCAL_OVERRIDE`, `LOG_LEVEL`, `HOST`, `PORT`, `USER`, `USERNAME`),
and 2 documented names are read through paths the plain grep misses (`LMSTUDIO_MODELS_DIR` via
`os.environ.get`, `SERVICE_USERPROFILE` by the service wrapper). The clone documents every name it reads.

Principle 24 problems in the source keys, all fixed by the clone: `lm_engine_COMPLETION_NUM_PREDICT`
is not UPPERCASE (`agent.py:403`); `HOST`/`PORT` are shadow-fallbacks behind `FLASK_HOST`/`FLASK_PORT`
(`574–575`) — the clone has one pair, `APP_HOST`/`APP_PORT`; the two percent keys are bare floats
(`ENTITY_MARKET_MOVE_PERCENT=2.0`, `ENTITY_FULL_URGENCY_MOVE_PERCENT=8.0`) and become
`"2.0%"` / `"8.0%"`, converted to fractions once in `AppConfig`. `LM_ENGINE_MAX_GENERATION=4096` is a
token cap, not a ratio, so it stays a bare number.

Two keys the source reads into constants and never uses (`MCTRADER_BASE_URL`, `MCTRADER_API_TOKEN` —
noted in the source's own Loose Ends) are **not** ported; the decision is recorded here.

**The clone's `.env.example` grows stage by stage, and a gate keeps it honest.** Stage 1 documents the 13
names its three booting pieces read (`STORAGE_ROOT`, `APP_HOST`, `APP_PORT`, `AI_MODEL`,
`LM_ENGINE_NORMAL_MODEL`, the seven `MCTRADER_MCP_*`, `LOG_LEVEL`) rather than 140 keys nothing reads
yet. `crates/shed-gates/src/settings.rs` fails the suite when a stage reads a name it has not documented,
documents a name no code reads, writes a value that is not the default the code produces, or spells a name
or a `_PERCENT` value the wrong way. `LOCALAPPDATA` and `HOME` are the two names on that gate's
`MACHINE_NAMES` allow-list: they locate the default storage root, they are set by Windows and the shell
rather than by the operator, and documenting them would advertise settings nobody is meant to set.
`APP_HOST`/`APP_PORT` replace all four source names, and the source's `FLASK_HOST`/`HOST` and
`FLASK_PORT`/`PORT` fallbacks are **not** read (§7 of the plan). Startup fails on an invalid value naming
the key; the unknown-key warning is deferred to Stage 9, where "unknown" becomes a true thing to say.

## 7. Deliberately absent, with reason

| Item | Reason |
|---|---|
| Routes 31-34 | nothing calls them; principle 16 |
| `list_recent_emails` / `read_email` and the IMAP helpers (`email_connection`, `extract_email_body`, `decode_mime_header`) | **decided**: out of scope for the clone; their five `EMAIL_*` keys are undocumented in the source |
| `MCTRADER_BASE_URL` / `MCTRADER_API_TOKEN` | the source reads them and never uses them; EODHD is reached only through McTrader |
| `USE_WAITRESS` toggle | **decided**: one server (tokio + axum); no dev-server mode, no dual-server key |
| Flask/Jinja templating | the frozen `index.html` needs only 3 substitutions; a substitution step replaces Jinja |
| Windows mode-switch launchers | already removed from the source |
| `tiktoken` token estimation | `utils/text.py` imports it in a `try/except` but **it is not in `requirements.txt`**, so `tiktoken = None` and the `len/4` fallback is the only path that actually runs. The clone implements that fallback directly; a Rust BPE port is out of scope. Revisit only if prompt-budget decisions prove wrong |
| `utils/eodhd_screener.py` | already deleted from the source (only `.pyc` remains) |

## 8. Fixed in the clone (improvements, each needs a test)

| # | Fix | Stage | Test exists |
|---|---|---|---|
| 1 | A goal can be closed; the entity writes what it did back to the goal | 7 | ☐ |
| 2 | `min_gap` and `turn_in_flight` vetoes are reachable | 7 | ☐ |
| 3 | A failed voice enumeration is an error, not an empty roster | 6 | ☐ |
| 4 | One `TTS_PROVIDER` default: `edge`, with Windows SAPI as the fallback provider (the source defaults to `windows` in `agent.py:557` while `.env.example` says `edge`) | 1 | ☐ |
| 5 | A printed reply records whether it was spoken | 6 | ☐ |
| 6 | A failed turn can say so out loud | 4 | ☐ |
| 7 | `streamed_whole` says when a reply arrived whole | 4 | ☐ |
| 8 | Pending trades survive a restart | 5 | ☐ |
| 9 | A voice tier has one authority in the server (JS mirror becomes a read of the server's value) | 6 | ☐ |
| 10 | Percentage config keys follow principle 24 | 7 (the two keys arrive with the mind stage; the **rule** is gated from Stage 1 — `settings::tests::every_documented_name_is_spelled_as_a_setting` rejects a `_PERCENT` key whose value is not double-quoted and ending in `%`) | ☐ |
| 11 | `.env` is never committed | 1 | ☑ `settings::tests::the_real_env_file_is_ignored_and_the_template_is_not` |
| 12 | `STORAGE_ROOT` default is not a swept temp directory | 1 | ☑ `storage::tests::the_default_root_sits_under_the_account_profile_not_the_repo` |
| 13 | A proactive line requires a live subscriber or a fresh poll | 7 | ☐ |
| 14 | `agent.py`'s 730 top-level nodes land in 18 crates + 1 binary with no god module | all | ☐ |
| 15 | Every env name the code reads is documented in `.env.example` (36 are undocumented today) | 1 (for the settings Stage 1 reads; the rest as each stage adds its own) | ☑ `settings::tests::every_setting_the_app_reads_is_documented`, `nothing_in_the_template_is_left_unread`, `every_documented_value_is_the_default_the_code_produces`, `every_documented_name_is_spelled_as_a_setting` |
| 16 | The audition route reports the audio it actually produced: `synthesize_speech_with_edge` writes `.mp3` while `/api/test-speak` hard-codes `audio/wav` (`agent.py:8637`), unlike `/api/speak` which sniffs it (`agent.py:3766`) | 6 | ☐ |

## 9. Stage gates (principle 20)

Every stage ends with all five green. The list below is the requirement; the record after it says what a
stage actually ran.

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `node --test "tests/*.js"` — `package.json` wires `npm test` to exactly this
- `cargo run -p shed-server -- --smoke` — Stage 1: boot, `GET /health`. `/` joins the smoke test at
  Stage 3 and one `/api/chat` turn against a fake engine at Stage 4, because neither route exists
  before the stage that owns it.

**Stage 1 — 2026-11-04.** fmt clean. clippy clean across all 20 packages under `-D warnings`.
`cargo test --workspace`: 76 tests — 50 in `shed-config`, 16 in `shed-gates`, 5 in `shed-http`, 5 in
`shed-logging`, 0 in the crates a later stage fills. `node --test "tests/*.js"`: 250 pass, 0 fail.
`--smoke` exits 0 and prints `smoke: GET /health on 0.0.0.0:5005 answered HTTP/1.1 200 OK with 256 bytes
of JSON`. No temporary or one-off file left in the repository or in the approved temporary tree.

Plus the stage-specific manual check written in the plan.

## 10. Stage 1 divergences, recorded while booting

Anything not on this list and not already in §7 is a parity bug.

| # | Divergence | Why | Where it is held |
|---|---|---|---|
| 1 | The `.env` file is read by the clone's own parser, not a crate | `dotenvy` rejects this operator's real `.env` — `LMSTUDIO_MODELS_DIR=D:\Models` is an illegal escape to it (`LineParse("D:\\Models", 3)`) and the whole file fails — where `python-dotenv` keeps the backslash and carries on. The clone implements `dotenv/parser.py`'s grammar: `export`, quoted keys, quoted and unquoted values, `# comment`, and a value that is nothing | `dotenv.rs`'s tests; the `single_env_reader` gate keeps a dotenv crate out of every other package |
| 2 | `APP_HOST`/`APP_PORT` only | §6: the source's `FLASK_HOST`/`HOST`/`FLASK_PORT`/`PORT` fallbacks name Flask, not the address | `server.rs`, `.env.example` |
| 3 | Log lines carry no ANSI codes and go to stderr | The source's Flask handler writes plain text to stderr and the service host redirects that stderr into the daily log (Stage 8). `tracing-subscriber`'s default features include `ansi`, so they are switched off: `default-features = false, features = ["fmt"]` | `shed-logging`'s tests, `Cargo.toml`'s pin comment |
| 4 | `RUST_LOG` is never read | The source has no such variable. `LOG_LEVEL` is the only level knob, and a second one would be a second authority for the same decision (principle 11) | `shed-logging` never builds an `EnvFilter` from the environment |
| 5 | `--smoke` binds `127.0.0.1:0` and then reports the configured `host:port` | The operator's Python app holds 5005 for the whole port, so the stage gate must not claim a port; the line it prints still names the address a real boot would bind | `bins/shed-server/src/smoke.rs` |
| 6 | The probe opens a real socket and does not half-close it | `Content-Length` is written by hyper while serving, not by the router, so only a served response proves the byte count; hyper's `half_close` defaults to false (`hyper-1.12.0/src/server/conn/http1.rs:260–271`), so a probe that shut its write half down would wait for an EOF that never arrives | `smoke.rs` |
| 7 | `/health`'s key order comes from `serde_json::Map` (a `BTreeMap`) and the body ends in one `\n` | Flask's `jsonify` sorts keys; `axum::Json` promises neither the order nor the trailing newline, and the captured response is compared byte for byte | `health.rs`'s 289-byte fixture |
| 8 | Non-ASCII text is **not** escaped | Flask's `app.json.ensure_ascii` defaults to `True`, so the Python app writes `\uXXXX` for a non-ASCII name; `serde_json` writes UTF-8. The `/health` payload is ASCII today, so the fixture cannot show the difference. Any route whose payload can carry a non-ASCII model, character or reply name is compared byte for byte when it is ported, and if the frozen client turns out to depend on the escapes, the clone escapes at that point | recorded here; each later route's parity test decides |
| 9 | `/health` carries no `X-Request-ID` | The source's request-id hooks are `/api/`-only (§1) | `health.rs`'s header assertion |
| 10 | Flask's `/health` failure path is not ported | The source's handler catches an exception and answers 500. The clone's `/health` reads only values `AppConfig` already validated at startup, so there is no failure left to catch inside the request; a bad setting stops the process before it can answer | `health.rs` |
| 11 | `--smoke` creates the real `%LOCALAPPDATA%\entity-in-the-shed` root | Stage 1 resolves and creates the storage root exactly as the app does, and the source creates its own at import time too (`agent.py:311–312`). The smoke run therefore touches the operator's real runtime directory — it writes nothing into it | recorded so the side effect is not a surprise; the directory is created by `Storage::resolve` |
| 12 | `mctrader_mcp.tool_count` is `0` and `tools` is `[]` until Stage 5 | `list_mctrader_mcp_tools()` opens a session against the MCP server, and `/health` may not depend on that. The rest of the block is byte-parity now | `health.rs`'s fixture, plan §12 decision 4 |
