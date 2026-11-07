# PLAN — Rust clone of Entity In The Shed

- **Source app:** `D:\Entity-In-The-Shed` — Python 3.13 / Flask, `agent.py` 11,607 lines + 24 pure `utils/` modules.
- **Target:** `D:\Entity_In_The_Shed` — Rust cargo workspace, one binary (`shed-server`), native only, no Python at runtime.
- **Rules:** `How_To_Code.md` governs everything below. Principle 21: this file is deleted when its coding is finished; deferred **coding** items move to `TODO/Loose-Ends.md`, each separated by a line of `********************************`.
- **Status:** planning only — no code written yet.
- **Provenance:** a previous attempt exists in git history (commit `ef90072`, deleted by `f43059c "start fresh"`). Its **decisions** are inherited (§1); its **facts** were re-derived from the source for this plan, and every number below was counted or read by this plan's author, not copied. Where an inherited claim could not be re-verified yet, §11 lists it with the check that will settle it. The old code may be **read** from git history as reference; nothing is copied from it.

## 0. Absolute-zero start

The workspace currently holds leftovers of the previous attempt. Stage 0 **deletes** them and rewrites each from this plan, so no line of code is inherited:

| Leftover | Stage 0 action |
|---|---|
| `bins/shed-server/` (main.rs, smoke.rs, Cargo.toml) | delete; the binary is rewritten at Stage 1 |
| `.env.example` | delete; rewritten at Stage 1, grown stage by stage |
| `package.json` | delete; rewritten at Stage 0 (`npm test` → `node --test "tests/*.js"`) |
| `.gitignore`, `.gitattributes`, `.vscode/` | keep — content is correct under D2; `.gitattributes` comments are reworded to stand alone (no dangling references to a deleted plan) |

## 1. Locked decisions

| # | Decision | Consequence |
|---|---|---|
| D1 | Phased parity, core first | Stages 0–9 (§8). Each stage ends with the stage gate (§8 preamble) green before the next starts. |
| D2 | Browser client reused byte-for-byte | `templates/index.html` (193,592 B), `static/app-helpers.js` (119,681 B), `styles/app.css` (11,673 B), `styles/information_info_1565.webp` (6,126 B) copied **unchanged** and frozen under a SHA-256 manifest gate. The Rust server must satisfy the existing HTTP/SSE contract exactly (§4). The 8 `tests/*.js` suites are copied unchanged and are the client gate — verified running on this machine (node v24: `node --test` passes). |
| D3 | Native Rust TTS | Hand-written Edge read-aloud WebSocket client + a Rust implementation of the v2 `Sec-MS-GEC` token, plus `Windows.Media.SpeechSynthesis` via the `windows` crate. The source's own `requirements.txt` comment is the evidence the v2 token is mandatory (v1 tokens are refused with a 403 handshake). Stage 6 opens with a time-boxed spike. |
| D4 | Cargo workspace, one crate per domain | Library crates mirroring the 24 `utils/` modules plus the domain groups of `agent.py`. `agent.py` is decomposed, not transliterated. **Improvement over the previous run:** a crate is *created* by the first stage that gives it a production caller — no empty crates (principle 16). The layering table (§3.3) is complete from day one and the gate enforces it over whatever crates exist. |
| D5 | Port the 49 pytest modules to Rust tests | Unit tests in each crate (`#[cfg(test)]` + `tests/`). HTTP tests drive the real router with `tower::ServiceExt::oneshot` and `reqwest`. The 8 JS suites stay unchanged. Routes the Python app left untested (restart, unload-models, keyring) get new Rust tests (principle 15). |
| D6 | Improving logic is in scope | Where the Python is tangled or defective, the Rust is written better — same observable behaviour, cleaner internals. Every deliberate divergence is a row in §7 and in `docs/PARITY.md`. |
| D7 | No Python at runtime, no shelling out to Python | The Windows service, the TTS path and the MCP client are all native Rust. |
| D8 | The Python app stays runnable until Stage 9 | Golden fixtures (route payloads, model reply documents, job `sentences[]` progress, real audio) are captured from the live Python app at Stages 1, 4, 6 and 7, before it is retired. Stage 9's side-by-side sweep is the final gate. |
| D9 | The Rust service is named **`entity_in_the_shed`** and deploys **alongside** the Python service, not as a replacement | Windows service name `entity_in_the_shed` (the Python installer's default is `entity-in-the-shed`, `install-entity-in-the-shed-service.ps1:28`), so both registrations coexist. Coexistence means no shared mutable state: the clone's default `STORAGE_ROOT` is `C:\Temp\entity_in_the_shed` (Python default `C:\Temp\entity-in-the-shed`, verified @305) — two live services never write the same JSON files. Both default to port 5005, so the Stage 8 installer must detect a port clash and require an explicit `APP_PORT` instead of failing at boot. Moving data onto the clone is a deliberate copy or a re-pointed `STORAGE_ROOT` with the old service stopped — never a live share. |

## 2. Verified source inventory

### 2.1 `agent.py` — 11,607 lines, 35 routes

Route table (every decorator read from the file; the 4 marked ✂ have **zero** references in `index.html` + `app-helpers.js` and are dropped — §7.1):

| Line | Route | Line | Route |
|---|---|---|---|
| 8418 | `GET /` | 8859 | `POST /api/speak` |
| 8433 | `GET /app.css` | 8944 | `GET /api/reply-laugh` |
| 8446 | `GET /app-helpers.js` | 8957 | `GET /api/reply-fart` |
| 8469 | `GET /styles/information_info_1565.webp` | 8986 | `POST /api/unload-models` |
| 8482 | `GET /api/countries` | 8997 | `POST /api/restart` |
| 8494 | `GET /api/voices` | 9389 | `GET /api/chat/jobs/<job_id>` |
| 8516 | `GET /api/characters` | 9443 | `POST /api/chat` |
| 8545 | `GET /api/models` | 11078 | `GET /api/entity/outbox` |
| 8565 | ✂ `GET /api/debug-voices` | 11100 | `POST /api/entity/outbox/<delivery_id>/ack` |
| 8602 | `POST /api/test-speak` | 11126 | `POST /api/entity/events` |
| 8650 | ✂ `GET /api/debug-models` | 11177 | `POST /api/entity/say` |
| 8665 | ✂ `GET /_routes` | 11207 | `POST /api/entity/proactive` |
| 8674 | ✂ `POST /api/set-model` | 11254 | `GET /api/entity/state` |
| 8702 | `GET /api/get-last-model-index` | 11473 | `GET /api/entity/stream` |
| 8717 | `POST /api/set-last-model-index` | 11502 | `GET /health` |
| 8792 | `GET /api/keyring` | 11595 | `POST /api/refresh-models` |
| 8797 | `POST /api/mcp/auth` | 8830 | `GET /api/mcp/tools` |
| 8846 | `POST /api/mcp/tools/refresh` | | |

Domain regions (anchors spot-checked by grep against the current file; the region map is the trace index for principle 25):

| Lines | Domain |
|---|---|
| 1–455 | imports, banner, `Flask` app + storage constants, local model map, LM Studio model + stop-token config |
| 458–596 | timeouts, mctrader + MCP config, sampler penalties, TTS + server + web-search config (`USE_WAITRESS` default **false** at 580; `APP_PORT` = `FLASK_PORT`→`PORT`→5005 at 575) |
| 598–702 | `BASE_SYSTEM_PROMPT`, prompt-template paths, delivery/tone TTS maps, `TOOL_DESCRIPTIONS`, broker number words |
| 704–777 | conversation retention, browser-chat job state, outbox + SSE config (`ENTITY_PROACTIVE_ENABLED` false @742, `ENTITY_SAY_ENABLED` false @746, `ENTITY_SSE_ENABLED` false @760) |
| 782–840 | slow-loop config (`ENTITY_SLOW_LOOP_ENABLED` false @782, `ENTITY_DRY_RUN` **true** @785) |
| 847–970 | user/fact/event/goal/mind-state config, caches, contextvars (`mcp_token_var`, `request_id_var`, …) |
| 973–1035 | HTTP plumbing: `api_error_response` @987, `api_json_boundary` @1002, request-id hooks @1016/@1024 (`/api/`-only), retryable-failure predicate |
| 1038–1236 | LM HTTP transport, retry ladder, streaming reader |
| 1239–1724 | model discovery from disk + API (`discover_lm_studio_models` @1529), `powershell_command` @1677 + `fetch_AI_MODEL_names` @1722 (dropped — §7.1), name/sex hints |
| 1727–2103 | model/background resolution, MCP config + session + tool layer |
| 2106–2193 | prompt templates, tone/delivery TTS mapping, browser model options |
| 2196–3085 | **character catalogue** (~890 lines — largest domain): countries, rosters, labels, fact sheets, profiles, outline, persona prompt |
| 3092–3276 | voice catalogue + selection |
| 3279–3358 | laugh/fart asset selection (`should_include_reply_laugh` @3348) |
| 3361–3473 | retention/cleanup + scheduler loop |
| 3476–3782 | TTS synthesis (`synthesize_speech` @3695), edge rate/pitch/volume strings, audio cache build, speech cache key |
| 3785–4174 | conversation records: paths, identity, load/save, delivery style, summarise, user details, history |
| 4185–4526 | housekeeping + profile workers |
| 4529–5000 | recall + persona guardrails (`enforce_persona_reply` @4886), append message |
| 5003–5073 | template/path rendering + `run_configured_command` |
| 5076–5225 | IMAP email — **not ported** (§7.1) |
| 5228–5501 | local tools + `TOOLS` dict (datetime, DuckDuckGo search/fetch, mctrader control, spreadsheet) |
| 5504–6031 | reply shaping + streaming: `JsonReplyStreamScanner` @5592, `SentenceSplitter` @5819, `ReplyStreamPublisher` @5988 |
| 6037–6202 | structured reply documents (`ast.literal_eval` acceptance — improved in Rust, §7.2) |
| 6207–6497 | router admin + residency (unload, model match, context windows, display names) |
| 6500–7131 | the completion call: `call_openai_api` @6500, prompt-block store, `call_lm_engine` @6808 (~324 lines) |
| 7134–7301 | timing + system-prompt assembly, force-web-search, broker-id extraction |
| 7304–7757 | mctrader tool-call inference from free text, screener selection, forced merge, temperature, reply segments |
| 7760–7956 | persistence + tool execution + recovery (`build_agent_follow_up_messages` @7935) |
| 7959–8417 | `run_agent` @7959 (~332 lines), ensure-complete/usable reply, duplicate-event guard |
| 8418–9510 | routes: pages/assets, catalogue, models, TTS, MCP/keyring, restart, browser-chat machinery (job store, busy payload, timing headers) |
| 9524–9644 | SSE machinery: `SseSubscriber` @9524, register/unregister, `publish_to_client`, `speak_to_client` @9600, `format_sse_frame` @9629 |
| 9647–10493 | mind store + reasoning (thoughts, user model, facts, goals, events, mind state, planner) |
| 10496–11050 | the slow tick (client records, background thought, utterance state, `run_entity_slow_tick`, mind loop worker) |
| 11053–11499 | entity routes + SSE response construction (`entity_stream_response` @11448, `api_entity_stream` @11473) |
| 11502–11607 | `healthcheck` @11503, `initialize_mctrader_credentials` @11519 (body is `pass` — not ported), `configure_logging` @11524, `run_http_server` @11553, `api_refresh_models` @11595, entry point @11606 |

### 2.2 `utils/` — 24 modules (all pure; none imports Flask or `agent`), line counts counted from the files

| Module | Lines | Crate | Module | Lines | Crate |
|---|---|---|---|---|---|
| `env.py` | 100 | shed-config | `mind_goals.py` | 361 | shed-mind |
| `json_util.py` | 66 | shed-json | `mind_state.py` | 453 | shed-mind |
| `text.py` | 552 | shed-text | `mind_user.py` | 567 | shed-mind |
| `characters.py` | 102 | shed-characters | `mcp.py` | 461 | shed-mcp |
| `models.py` | 170 | shed-models | `mcp_args.py` | 273 | shed-mcp |
| `lm_router.py` | 136 | shed-models | `tts.py` | 184 | shed-tts |
| `prompt.py` | 164 | shed-prompts | `tts_cache.py` | 506 | shed-tts |
| `prompt_cache.py` | 154 | shed-prompts | `speech_pauses.py` | 291 | shed-tts |
| `conversation_history.py` | 445 | shed-conversation | `reply.py` | 389 | shed-chat |
| `entity_store.py` | 259 | shed-store | `pending_trades.py` | 311 | shed-tools |
| `entity_client.py` | 127 | shed-entity | `mind_outbox.py` | 363 | shed-entity |
| `entity_loop.py` | 548 | shed-mind | `mind_events.py` | 483 | shed-mind |

Cross-imports inside `utils/`: `json_util→text`, `mcp→env`, `mcp→json_util`, `conversation_history→entity_store`, `mind_outbox→entity_store`, `prompt→models`. Only `pending_trades.py` holds module-level mutable state.

### 2.3 Assets, data, tests

- **Frozen client (D2):** the 4 files in §1 D2. `index.html` is rendered with **exactly three** substitutions, all Jinja `|tojson` (verified by grep): `chat_request_timeout_ms` (`LM_ENGINE_REPLY_TIMEOUT * 1000`, or `None`), `model_selection_requires_password`, `browser_tts_enabled` (`bool(TTS_BROWSER_FALLBACK)`) — render call at 8420–8425.
- **Data read at runtime:** `characters/` (110 country files, 110 counted), `voices.json`, `prompts/` (7 files — `system_prompt.md`, 4 delivery prompts, 2 authoring aids; the whole folder is copied), `outputs/tts/{laugh,fart}/` audio.
- **Tests:** 49 `tests/test_*.py` + `conftest.py`; 8 `tests/test_*.js` browser suites (all eight require `../static/app-helpers.js`, verified — hence the mirrored `tests/` + `static/` layout; `node --test` runs them with zero dependencies, proven on this machine: 62/62 green for two suites sampled).
- **`last_model_index.txt`** sits in the Python repo root; the clone moves it under `STORAGE_ROOT` (§7.1).
- **Python deps → Rust:** Flask+waitress→`axum`+`tokio`; `requests`→`reqwest`; `python-dotenv`→see Stage 1 spike (§11 U1); `mcp>=1.27`→`rmcp` or hand-written JSON-RPC stdio; `edge-tts>=7.3`→hand-written WebSocket + v2 token (D3); `pywin32`→`windows` crate; `keyring`→`keyring` crate; `waitress`→dropped (D-ops, §7.1).

### 2.4 Runtime shape and defaults

`if __name__ == "__main__": run_http_server()`. Startup order (read from `run_http_server`, 11553–11592): `configure_logging` → `initialize_mctrader_credentials` (a `pass` — not ported) → `discover_lm_studio_models` → `start_cleanup_scheduler` → `start_conversation_housekeeper` → `start_character_catalog_warmer` → `start_mind_loop` → banner `print` @11577 → server.

The server default, verified at 578–592: **`USE_WAITRESS` defaults false** (580) and the source's own `.env.example` ships `USE_WAITRESS=0`, so the app normally runs on **Werkzeug threaded** (`app.run(..., threaded=True)` @11592); `waitress.serve(threads=32, channel_timeout=600)` @11583–11589 is the opt-in. (The source README's "runs under Waitress" line contradicts its own defaults — a source-doc bug, not a clone problem.) The clone has **one** server (tokio + axum) sized like the waitress branch — bounded workers, a channel/read timeout — because that is what a long-lived SSE stream needs.

Daemon threads in the source: cleanup-scheduler, conversation housekeeper, character warmer, mind-loop, per-request browser-chat jobs, conversation-profile workers. All become named tokio tasks owned by the binary.

Everything unprompted is **off by default** — all five verified this session: `ENTITY_PROACTIVE_ENABLED=false` (742), `ENTITY_SAY_ENABLED=false` (746), `ENTITY_SSE_ENABLED=false` (760), `ENTITY_SLOW_LOOP_ENABLED=false` (782), `ENTITY_DRY_RUN=true` (785). `/api/entity/state` publishes the flags; the client stands its stream up or down. The clone keeps the same defaults and the same opt-in chain; Stage 7's exit gate runs with the chain deliberately switched on **and** with the defaults to prove the off state is unchanged.

`configure_logging` (11524–11550): `LOG_LEVEL` matched case-insensitively against `DEBUG/INFO/WARNING/ERROR`; anything else — blank or typo — falls back to INFO; only the app's own logger is raised, library chatter stays at the root; the startup line names the live level.

### 2.5 Configuration — counted this session

- **152 distinct env names** are read across `agent.py` + `utils/` + `scripts/` (148 in `agent.py`; `utils/` adds `MCTRADER_MCP_SERVER_NAME`, `USER`, `USERNAME`; `scripts/` adds `SERVICE_NAME`). Counted by matching a literal UPPER_CASE name as the first argument of `os.getenv` / `getenv_int` / `getenv_float` / `getenv_timeout_seconds` / `parse_json_env` / `os.environ.get`. `parse_bool` calls wrap `os.getenv` and are already inside those counts. The counting script is reproducible; re-run it before Stage 9's config sweep.
- The source's `.env.example` documents **113** names: **40 read-but-undocumented** (including `EMAIL_*` ×5, `LOG_LEVEL`, `HOST`, `PORT`, `LM_ENGINE_REPLY_TIMEOUT`, `MCTRADER_MCP_TIMEOUT`, `MCTRADER_MCP_CACHE_TTL`, `MCTRADER_DEFAULT_*`, `TTS_TIMEOUT`, `TTS_DEFAULT_RATE`, `WEB_SEARCH_ENABLED`, `CLEANUP_INTERVAL_SECONDS`, all `*_RETENTION_DAYS`, `ENTITY_MAX_UTTERANCES_PER_{HOUR,DAY}`, `ENTITY_MIN_UTTERANCE_GAP_SECONDS`, `ENTITY_REPEAT_GUARD_LINES`, `BROWSER_CHAT_JOB_{TTL_SECONDS,MAX_SENTENCES}`, `LM_ENGINE_RETRY_{BASE,JITTER}_MS`, `LM_ENGINE_TRANSIENT_RETRIES`, `LM_ENGINE_MODEL_ADMIN_TIMEOUT`, `LM_ENGINE_RESIDENCY_PROBE_TIMEOUT`, `MODEL_FALLBACK_TIMEOUT_SECONDS`, `AI_MODEL_DISCOVERY_TIMEOUT`, `AI_MODEL_LOCAL_OVERRIDE`, `USERPROFILE`, `lm_engine_COMPLETION_NUM_PREDICT`) and **1 documented-but-not-read** in any greppable way (`SERVICE_USERPROFILE`, consumed by the service wrapper — re-verify when Stage 8 ports it). These undocumented 40 are the ones most likely to be mis-set; documenting every name the clone actually reads is the point.
- **Principle-24 violations found in the source:** `lm_engine_COMPLETION_NUM_PREDICT` (403) is not UPPERCASE; `HOST`/`PORT` are read as fallbacks behind `FLASK_HOST`/`FLASK_PORT` (574–575); the two percent keys (`ENTITY_MARKET_MOVE_PERCENT`, `ENTITY_FULL_URGENCY_MOVE_PERCENT`) are bare floats.
- **Principle 24 in the clone:** every key UPPERCASE; percentage keys end `_PERCENT` with a double-quoted value ending in `%` (`ENTITY_MARKET_MOVE_PERCENT="2.0%"`), parsed into `f64` fractions once at startup; `APP_HOST`/`APP_PORT` only (the four source names are not read). The clone's final `.env.example` ≈ 140 keys, every one actually read.

## 3. Target architecture

### 3.1 Workspace layout

```
Entity_In_The_Shed/
├── Cargo.toml                [workspace] + [workspace.dependencies] + [workspace.lints]
├── rustfmt.toml              max_width = 100, stable-channel options only
├── clippy.toml               too-many-lines-threshold = 50 (principle 5)
├── package.json              npm test → node --test "tests/*.js"
├── .env.example              grown stage by stage, every name the code reads (§2.5)
├── .gitignore / .gitattributes (kept, comments reworded — §0)
├── README.md
├── docs/
│   ├── ARCHITECTURE.md       crate map + dependency direction + ownership (§3.2/§3.3)
│   ├── CONTRACT.md           the frozen HTTP/SSE contract (§4), treated as the spec
│   ├── PARITY.md             parity table + divergences (§7), kept current
│   └── GUARD_CATALOGUE.md    principle-23 index
├── templates/index.html      frozen (D2)   ├── characters/  prompts/  voices.json
├── static/app-helpers.js     frozen (D2)   ├── outputs/tts/{laugh,fart}/
├── styles/                   frozen (D2)   ├── crates/      (created on demand, D4)
├── bins/shed-server/         the one binary (Stage 1)
└── tests/
    ├── test_*.js             the 8 browser suites, unchanged (D2)
    └── assets.manifest.json  path + bytes + sha256 of every frozen file
```

The layout mirrors the Python app on purpose: the JS suites `require("../static/app-helpers.js")`, so a mirrored `tests/`-next-to-`static/` layout needs zero client edits.

### 3.2 Crates (16 domain + 2 dev-only + 1 binary; created on demand per D4)

| Crate | Source of its logic | Responsibility |
|---|---|---|
| `shed-config` | `utils/env.py` + `agent.py:376–955` | Typed `AppConfig`: every env name parsed once, validated, immutable. No other crate reads env. |
| `shed-json` | `utils/json_util.py` | JSON safety, strict response parsing, token-byte helpers |
| `shed-text` | `utils/text.py` | token estimation, tone-tag/markup normalisation, vocalisation respelling, bracket segmentation |
| `shed-store` | `utils/entity_store.py` | atomic JSON/JSONL writes, quarantine + recovery, client-id mint/normalise |
| `shed-logging` | `configure_logging` (11524–11550) | one logger, one format, one destination; owns the `LOG_LEVEL` mapping; never reads `RUST_LOG` |
| `shed-models` | `utils/{models,lm_router}.py`, `agent.py:1239–1724`, `6207–6497` | discovery (disk + API) + cache, context windows, display names, router unload/residency |
| `shed-llm` | `agent.py:1038–1236`, `6500–7131` | LM Studio/OpenAI-compatible transport: retry ladder, streaming reader, sampler payload, prompt-block store, response parsing |
| `shed-prompts` | `utils/{prompt,prompt_cache}.py`, `agent.py:598–702`, `2106–2193`, `7134–7247` | templates, tone/delivery maps, combined system prompt, prompt-cache plan |
| `shed-characters` | `utils/characters.py`, `agent.py:2196–3085` | country/character catalogue, profiles, outlines, sex/pronouns, roster disambiguation |
| `shed-conversation` | `utils/conversation_history.py`, `agent.py:3785–5000` | records, identity/filenames, compaction, summarisation, housekeeping, recall + persona guardrails |
| `shed-chat` | `utils/reply.py`, `agent.py:5504–6202`, `7760–8417` | reply shaping, JSON reply scanner, sentence splitter, stream publisher, `run_agent`, completeness/usability recovery |
| `shed-tools` | `agent.py:5003–5073`, `5228–5501`, `utils/pending_trades.py` | local tool registry + implementations, DuckDuckGo search/fetch with private-address block, spreadsheet writer, configured start/stop/status commands, trade preview + confirmation state machine |
| `shed-mcp` | `utils/{mcp,mcp_args}.py`, `agent.py:1727–2103`, `7304–7757` | MCP session/transport, tool normalisation, auth gating, redaction, free-text argument inference, mctrader tool-call resolution |
| `shed-tts` | `utils/{tts,tts_cache,speech_pauses}.py`, `agent.py:3092–3276`, `3476–3782` | voice catalogue + selection, Edge WebSocket synthesis (D3), Windows SAPI synthesis, audio cache, MP3 pause trimming, failure description |
| `shed-entity` | `utils/{entity_client,mind_outbox}.py`, `agent.py:9524–9644`, `10496–10602`, `11053–11107` | client records, outbox store, SSE subscriber registry, frame formatting, utterance history |
| `shed-mind` | `utils/{entity_loop,mind_events,mind_goals,mind_state,mind_user}.py`, `agent.py:9647–10493`, `10796–11050` | slow loop, ~20 vetoes, grounding, events, goals, drives, user model, thought journal |
| `shed-http` | `agent.py:8418–9510`, `11053–11607` | axum router (31 of 35 routes — §7.1 drops 4), middleware, error envelope, static assets, deferred chat jobs |
| `shed-testutil` | — | dev-only: temp storage roots, fake LM/MCP/Edge endpoints, fixture builders |
| `shed-gates` | — | dev-only: workspace-wide invariant gates (§6): frozen assets, crate layering, env-read rule, settings template, guard-catalogue honesty |
| `shed-server` (bin) | `agent.py:11553–11592` | startup order, background tasks, graceful shutdown, `--smoke` boot gate |

### 3.3 Dependency direction (never violated; gate-checked)

```
shed-config → (nothing)          shed-json, shed-text, shed-logging → shed-config
shed-store → shed-config, shed-json
shed-models, shed-prompts, shed-characters → config, json, text, store
shed-llm → config, json, text, models, prompts
shed-conversation → config, json, text, store, models, prompts
shed-tools, shed-mcp → config, json, text, store
shed-chat → config, json, text, models, llm, prompts, tools, mcp, conversation
shed-tts → config, text, store
shed-entity → config, json, store
shed-mind → entity, models, llm, prompts, conversation, tools
shed-http → the 16 above (composition root, apart from the binary)
shed-server → everything, including shed-http
shed-testutil → the 16 above (dev-dependency only)
shed-gates → shed-config, shed-testutil (dev-dependency only; no shipped crate may depend on either)
```

No cycles; no crate depends on a crate that composes it. The table is `LAYERS` in `crates/shed-gates/src/layers.rs`; the gate reads each existing package's `Cargo.toml` against it and enforces only what exists (D4).

## 4. The frozen contract

Everything here was read from the source this session (line numbers cite the reading). §4 is copied to `docs/CONTRACT.md` and treated as the spec; the client is never edited, so any Rust change here is a client break.

### 4.1 Cross-cutting

- **Error envelope** (`api_error_response` 987–999): `{"ok": false, "error": str, "code": str, "request_id": str}`, status as chosen by the handler, plus the `X-Request-ID` header on the response. `extra` dicts merge in.
- **Boundary** (`api_json_boundary` 1002–1012): any unhandled handler exception → `500` / `code: "internal_error"` / message `"internal server error"`, traceback logged with the request id. Rust: a `Result`-returning handler wrapper; **no panics reach the client**. Every `/api/*` route is boundary-wrapped **except** `/api/entity/stream` (the source explains why at 11477–11480: a JSON boundary cannot catch a generator error after the response object is returned — Rust keeps the same rule: SSE errors are handled inside the stream).
- **Request id** (1015–1027): assigned in `before_request` and echoed in `after_request` **only for paths starting `/api/`** — which is why `/health` carries no `X-Request-ID`. One axum middleware layer reproduces the scoping exactly.
- **Chat timing headers**: `X-Entity-In-The-Shed-Timing` (summary truncated to 1000 chars) and `X-Entity-In-The-Shed-Total-Ms` (`"{:.1}"`-style one-decimal milliseconds).
- **Cache headers** on `/`, `/app.css`, `/app-helpers.js`, the webp: `Cache-Control: no-store, no-cache, max-age=0, must-revalidate`, `Pragma: no-cache`, `Expires: 0`.
- **`index.html` renders with exactly three substitutions** (§2.3); a tiny substitution step replaces Jinja (no template engine dependency).

### 4.2 `POST /api/chat` (9443–9510) and `GET /api/chat/jobs/<job_id>` (9389–9440)

- Body JSON; `deferred` parsed with the `parse_bool` equivalent, default `false`. Missing/empty `message` → `400` `code: "message_required"`.
- **Busy path:** when the single conversation slot cannot be taken non-blockingly → **HTTP 200** `{"ok": true, "reply": "The Entity is in consultation with another, please try later."}`. Not 429. (Text verified verbatim.)
- **Deferred path:** **HTTP 202** `{"ok": true, "pending": true, "job_id": <32-hex>, "status": "pending"}`; the job is registered `{status: "pending", created_at, expires_at: now + BROWSER_CHAT_JOB_TTL_SECONDS, sentences: []}` and runs on a worker; a spawn failure pops the job, releases the slot and surfaces the error.
- **Job status — three shapes, all load-bearing:**
  - unknown/expired → `404` `code: "chat_job_not_found"`;
  - pending → `{ok, pending: true, job_id, status, delivery_style, sentences[], sentence_count}`, where `delivery_style` is `""` until the worker resolves the turn's style (the page voices streamed sentences from it — empty is meaningful, not missing);
  - complete → the stored `response_payload` **with `sentences` re-attached** plus the timing headers; a stored failure replays the recorded `http_status` / `error_code` / message. The re-attachment exists because the turn publishes its closing sentence after the last poll that could have carried it (comment at 9400–9405); the client de-duplicates.
- **Single-slot rule** (`try_acquire_browser_chat_slot` + `AI_CONVERSATION_LOCK`): one conversation turn at a time, process-wide. Rust: one `TurnGate` owner (§7.2) with a test that reproduces the busy payload under two simultaneous requests. Slot release stamps `LAST_TURN_ENDED_AT` — the slow loop reads it.

### 4.3 Entity outbox + SSE

- `speak_to_client` (9600–9626) is **the single door**: every spoken line is queued (durable half) **and** pushed (live half); the docstring explains why both halves are mandatory. Rust keeps one door.
- **Frame format** (`format_sse_frame` 9629–9640): optional `id: ` line, then `event: `, then one `data: ` line **per line of the JSON payload** (multi-line payloads split; `ensure_ascii=False`), terminated by a blank line.
- **Stream body** (`entity_stream_response` 11448–11470): first frame `event: state` with `{"client_id": ..., "ok": true}`, then catch-up `utterance` frames, then live frames with `": hb\n\n"` comment heartbeats, ending at `ENTITY_SSE_MAX_LIFETIME_SECONDS`. Headers: `Cache-Control: no-cache`, `X-Accel-Buffering: no`, and **`Connection` must not be set** — the source's own comment (11460–11465) explains waitress raises `AssertionError` for a WSGI app setting a hop-by-hop header; the Rust server likewise leaves it to the HTTP layer.
- **Refusals** (11482–11495): `404 entity_stream_disabled` (flag off), `429 entity_reconnect_too_fast`, `503 entity_stream_full`. Cursor from `?since=` or the leading integer of `Last-Event-ID` (ids are `<seq>-<ms>`).
- **Registry semantics** (9524–9597, all load-bearing): a reconnect for the same `client_id` **replaces and closes** the old subscriber (the cap bounds distinct clients, not connections); unregister is identity-checked so a departing old stream cannot evict its replacement; a full per-subscriber queue **drops the oldest** frame and keeps the newest (a wedged client must never stall the publisher); `close()` unblocks the parked generator.
- Outbox view (`outbox_message_payload` ~11053–11075): `seq`, `delivery_id`, `kind`, `trigger`, `text` (markup-normalised for reading), `text_markup` (raw, only when it differs), `character_slug`, `delivery_style`, `created_at`. `GET /api/entity/outbox` **drains**; a message stays deliverable until acked; `POST .../ack` is idempotent `{ok, delivery_id, changed}`. (Payload-level details are re-read line-by-line at Stage 7.)

### 4.4 `POST /api/speak` (8859–8941)

Request fields `text, voice, provider, character_id, preferred_language, delivery_style, tone` — **`rate`/`pitch` are not read**; they come from the delivery/tone TTS maps. Success: audio file with its mimetype + `X-TTS-Voice`, `X-TTS-Provider`, `X-TTS-Cache: hit|miss`, `Cache-Control: no-store`. Cache hits come from the durable cache and are **not deleted**; misses are served from a temp folder deleted shortly after serving. Failure: `describe_tts_failure(...)` message with `500` / `code: "tts_failed"` — the message text is contract because the client displays it. (Read line-by-line at Stage 6 before porting.)

### 4.5 Client invariants that pin the server

The deferred-chat flow (202 + job polling), sentence-by-sentence streaming from `sentences[]`, the busy reply text, the outbox poll standing down while the SSE stream is up, `client_id` minting/echoing, and the three template substitutions are all load-bearing. Any Rust-side "improvement" that changes one of these is out of scope for the clone and goes to `TODO/Loose-Ends.md`.

## 5. Configuration policy

- `.env.example` grows stage by stage: every name the stage's code reads is documented in that stage, grouped by owning crate, each with its real default; the settings gate (§6) fails if read-set and documented-set diverge (allow-list: `LOCALAPPDATA`, `HOME`, `USER`, `USERNAME`, `USERPROFILE` — machine-set, not operator-set).
- Startup validates the whole config before any task starts (principle 14): invalid value → hard failure naming the key; `STORAGE_ROOT` (clone default `C:\Temp\entity_in_the_shed`, D9) is resolved and created before anything else boots. The **unknown-key warning lands at Stage 9**, when the read-set is final — until then a warning would fire on keys of unported stages.
- Secrets (`OPENAI_API_KEY`, mctrader login) come from the environment or the Windows credential store (`keyring` crate), never from a file in the repo. `GET /api/keyring` reports the process account and whether a credential exists — never the credential.
- The five unprompted-entity flags keep their source defaults (§2.4). A Stage-7 demo that "the entity speaks on its own" is a configured state, and the config that produces it is written down.

## 6. `How_To_Code.md` → Rust gates

| Principle | Enforcement in this workspace |
|---|---|
| 1 | One implementation per behaviour; Python never called at runtime; no `#[deprecated]` shims, no compat routes (§7.1). Fallbacks only where principle 1 allows: external systems (LM Studio, Edge TTS, mctrader, DuckDuckGo). |
| 2, 5 | `clippy::too_many_lines` denied, `clippy.toml: too-many-lines-threshold = 50`. An exceedance needs an `#[allow]` + one-line justification, reviewed at the stage gate. |
| 3, 12 | `serde` structs/enums for every request/response/record/tool payload. `serde_json::Value` banned outside `shed-json` and the MCP argument layer, with one recorded exception: `shed-models` reads an engine's discovery document as `Value`, because the shapes `parse_discovery_candidates` accepts are the engine's, not the app's (§3.2, Stage 2). Newtypes: `ClientId`, `DeliveryId`, `JobId`, `CharacterId`, `ModelName`, `Seconds`, `Percent`. |
| 4, 10 | The crate map §3.2 + direction §3.3. `agent.py`'s 730 top-level nodes become collaborators behind small facades (`OutboxStore`, `TtsAudioCache`, `MindState`, `ToolRegistry`, `LlmClient`, `TurnGate`). |
| 6 | Crate-private modules; `pub` only what another crate needs; `#[cfg(test)]`-only accessors. |
| 7 | The 24 `utils/` modules port as free functions; state lives in named owners, never globals. |
| 8 | Each crate splits pure logic from I/O (`reqwest`/`tokio`/fs). HTTP tests run against fakes in `shed-testutil`, never the real LM Studio. |
| 9 | `build_ / validate_ / load_ / save_ / parse_ / normalize_ / select_ / compute_`; names containing `and_` are a review flag. |
| 11, 19 | Each constant/rule exists in exactly one crate; `docs/ARCHITECTURE.md` records the owner. Duplicated vetoes/normalisations in `agent.py` collapse into single implementations. |
| 13 | Tests assert HTTP responses, JSON payloads, file contents, function outputs — never private helpers. The 8 JS suites test the client against the real server from Stage 3 on. |
| 14 | Config validation, storage-root creation, model availability and voice selection all precede a model call or a synthesis. |
| 15 | §9 maps every Python test module to a Rust test; a crate with no test fails its stage gate. Untested Python routes (restart/unload-models/keyring) get new Rust tests. |
| 16 | `[workspace.lints]`: `deny` on `dead_code`, `unused`, `unused_crate_dependencies`, `unsafe_code`, `clippy::all`, `clippy::unwrap_used`, `clippy::expect_used` (test targets included — test helpers return `Result`). `clippy::pedantic` is not a group (it would import lints this project does not want); wanted pedantic lints are named individually. `cargo clippy --workspace --all-targets -- -D warnings` is a stage gate. D4's create-on-demand is the same rule in `Cargo.toml` form. |
| 17 | I/O functions return results and are named `load_/save_/append_/publish_`; no function mutates a global; shared state is a named owner passed in. |
| 18 | `rustfmt.toml: max_width = 100`; comments explain *why* — several source docstrings are load-bearing design notes and are kept as Rust doc-comments. |
| 20 | Every stage gate (§8 preamble) runs `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `npm test`, the parity checklist; temp/one-off scripts are deleted before the stage closes. |
| 21 | At Stage 9 this file is deleted; deferred coding items go to `TODO/Loose-Ends.md` separated by `********************************`. |
| 22 | The clone stores JSON/JSONL, not tables: bounded persisted fields keep their source limits (`MAX_TEXT_CHARS`, `MAX_GOAL_TEXT_CHARS`, `MAX_EVENT_TEXT_CHARS`); nothing unbounded is persisted. |
| 23 | `docs/GUARD_CATALOGUE.md`: one entry per guard (name, owning crate+function, what it gates, what fires/clears it, config key, tests, ~5-line note). Known guards: the ~20 `utils/entity_loop.py` vetoes, speech-budget caps, quiet hours, SSE reconnect throttle, subscriber cap, tool auth/arming gates, trade confirmation + code expiry + price-move refusal, `is_public_web_url` private-address block, `enforce_persona_reply`, the repeat guard, outbox TTLs, the busy slot. `shed-gates/src/guards.rs` fails on a stale entry location or a known guard with no entry. |
| 24 | §2.5 / §5. |
| 25 | Read the affected Python region + its call chain before porting; §2.1 is the trace index. |

## 7. Deliberate divergences

Anything not on this list is a parity bug; each row is also a row in `docs/PARITY.md`.

### 7.1 Surfaces the clone does not carry

| Surface | Why | Evidence (verified) |
|---|---|---|
| `GET /_routes`, `GET /api/debug-voices`, `GET /api/debug-models`, `POST /api/set-model` | Flask introspection artifacts / duplicates / legacy mode-switch leftover | 0 references in `index.html` + `app-helpers.js`, counted this session |
| `MCTRADER_BASE_URL`, `MCTRADER_API_TOKEN` | read once, used nowhere else | re-grep at Stage 5 before dropping |
| `initialize_mctrader_credentials` | body is `pass` | read @11519 |
| `powershell_command` + `fetch_AI_MODEL_names` | PowerShell model discovery; `discover_lm_studio_models_from_disk` is the native path the app depends on | read @1677/@1722 |
| `last_model_index.txt` in the repo root | moved under `STORAGE_ROOT` | principle 19 |
| IMAP email tools (`agent.py:5076–5225`) + the five `EMAIL_*` keys | separate concern; keys read by the code and documented nowhere in the source | in §2.5's undocumented list |
| `USE_WAITRESS` / `WAITRESS_THREADS` / `WAITRESS_CHANNEL_TIMEOUT` as a toggle | one server: tokio + axum sized like the waitress branch (32 workers, 600 s channel timeout) | §2.4 |
| Flask/Jinja rendering | the frozen page needs exactly 3 substitutions | §2.3 |

`POST /api/test-speak` **is** ported despite 0 client references: it is the voice-audition surface `MARKUP_TESTING.md` describes and 5 Python test references pin its behaviour (§2.3 route table, Stage 6). `/api/restart` is client-referenced; `/api/keyring` and `/api/unload-models` are documented operator surfaces with no tests — all three port at Stage 8 with **new** Rust tests.

### 7.2 Structural improvements allowed by D6

| Change | Reason |
|---|---|
| One `AppConfig` replaces 148 scattered env reads and their derived constants | principle 19 |
| `run_agent` (~332 lines) and `call_lm_engine` (~324 lines) decompose into plan → call → parse → recover → persist | principles 2/5 |
| The retry ladder (fallback model, temperature, tools, json-mode, reasoning-effort, max-completion-tokens) becomes one ordered `RetryStrategy` chain | principle 11 |
| The ~20 slow-loop vetoes become named guard functions with catalogue entries | principle 23 |
| `synthesize_speech` (~88 lines) splits into voice choice → provider call → audio validation → cache admission, with one delete path | principles 2/17 |
| Reply-document parsing (`ast.literal_eval` acceptance) becomes strict JSON with an explicit, catalogued repair step | principles 3/18 |
| Conversation lock + chat slot + active-job counter + `LAST_TURN_ENDED_AT` become one `TurnGate` owner | principles 6/17 |
| Crates created on demand; gates enforce the full layer map over whatever exists | principle 16 (D4) |
| Service name `entity_in_the_shed` and default storage root `C:\Temp\entity_in_the_shed` differ from the Python app's `entity-in-the-shed` / `C:\Temp\entity-in-the-shed` | coexistence (D9): two live services must not share a service registration, a port, or mutable state on disk |

### 7.3 What must **not** change

§4's contract, the queue-**and**-push delivery door, the single-turn gate, the 202 + job-polling flow, the busy reply text, the `sentences[]` shape and re-attachment, the `text` vs `text_markup` split, the SSE registry semantics, and the frozen client files.

## 8. Stages

**Stage gate (every stage):** `cargo fmt --check` → `cargo clippy --workspace --all-targets -- -D warnings` → `cargo test --workspace` → `npm test` (JS suites) → `docs/PARITY.md` updated → guard catalogue updated → no temp files left → commit.

| Stage | Deliverable | Extra exit gate |
|---|---|---|
| **0** | §0 deletions; workspace root (`Cargo.toml` + lints + `rustfmt.toml` + `clippy.toml`), `package.json` rewritten, `docs/` skeleton, client + data copied byte-for-byte into the mirrored layout with `tests/assets.manifest.json` (path+bytes+sha256), `shed-gates` (frozen-asset, layering, env-read gates), `.gitattributes` comments reworded | asset gate passes over every frozen file; the 8 JS suites pass unchanged via `npm test`; dependency-resolution probe: `cargo add --dry-run` each planned external crate to confirm it resolves on this machine (records §10 risk 10) |
| **1** | **Spike first (§11 U1):** test dotenvy against the operator's real `.env`; then `shed-config` (typed `AppConfig`, storage root, log level, model names, MCP config slice), `shed-logging`, `shed-http` (`GET /health` byte-parity against a fixture captured from the live Python app), the `shed-server` binary (startup order §2.4 minus the not-yet-ported tasks, `--smoke` self-answer gate), `.env.example` for exactly what Stage 1 reads, settings gate | `/health` byte-comparable to the captured fixture; `test_env_utils` ported; `--smoke` exits 0 while the Python app still holds port 5005 (binds `127.0.0.1:0`) |
| **2** | `shed-models` + `shed-llm`: discovery (disk + API), cache persistence, context windows, transport + retry ladder + streaming reader, prompt-block store, router admin/residency | `test_models_utils`, `test_lm_router_utils`, `test_agent_models`, `test_lm_streaming`, `test_json_util_utils`, `test_text_utils` (model-name parts), `test_http_server` (boot-path parts) ported; a fake-LM test proves the retry ladder |
| **3** | `shed-prompts` + `shed-characters`; `/`, `/app.css`, `/app-helpers.js`, webp, `/api/countries`, `/api/characters`, `/api/voices`, `/api/models`, last-model-index pair, `/api/refresh-models`; warmer task; `X-Request-ID` middleware + `/api/*` error envelope | the frozen client loads and the country→character→voice flow works end-to-end against the real `characters/` data; the JS suites run against the Rust server from here on; `test_prompt_utils`, `test_prompt_cache`, `test_characters`, `test_character_profiles`, `test_app_stylesheet`, `test_frontend_cache_headers` ported |
| **4** | `shed-conversation` + `shed-chat`: records, identity, compaction, summarisation, housekeeping, recall/persona guardrails, reply shaping, scanner, splitter, publisher, `run_agent`, `TurnGate`, `POST /api/chat` + job route. **Captures golden fixtures from the live Python app** (chat turns, glm4 model output, job `sentences[]` progress) | `test_api_chat`, `test_conversation_*` (3), `test_reply_*` (4), `test_sentence_splitter`, `test_glm4*`, `test_entity_store_utils`, `test_text_utils` (reply parts) ported; a chat turn streams sentences to the frozen client; the busy path reproduces under two simultaneous requests |
| **5** | `shed-tools` + `shed-mcp`: local tools, DuckDuckGo search/fetch, spreadsheet writer, trade preview + confirmation, MCP client (stdio + sse/streamable-http), mctrader inference + auth gating, `/api/mcp/{auth,tools,tools/refresh}`; `/health`'s mctrader block completes (tool count/list) | `test_mcp_*` (3), `test_pending_trades`, `test_trade_confirmation_flow` ported; a tool call round-trips through a fake MCP server |
| **6** | **Opens with the time-boxed `Sec-MS-GEC` v2 spike** (§10 risk 1; fallback: ship Windows-SAPI-only, record Edge as a Loose-End, keep the failure-message contract). Then `shed-tts`: voice catalogue, Edge WebSocket synthesis, Windows SAPI, audio cache, MP3 pause trimming, failure description → `/api/speak`, `/api/test-speak`, `/api/reply-laugh`, `/api/reply-fart`. Captures real audio fixtures | `test_tts_*` (4), `test_speech_pauses`, `test_voice_tones`, `test_test_speak_route`, `test_text_utils` (markup parts) ported; a cached phrase answers `X-TTS-Cache: hit` |
| **7** | `shed-entity` + `shed-mind`: client records, outbox, SSE registry + stream, slow loop, events, goals, drives, user model → all `/api/entity/*` routes. Captures mind-loop tick fixtures | `test_entity_*` (3), `test_mind_*` (9 incl. the three `_wiring` suites), `test_outbox`, `test_event_stream` ported; the entity speaks unprompted through SSE with the opt-in chain switched on (config written down), **and** a second run on the defaults proves the off state unchanged |
| **8** | Ops: `/api/restart`, `/api/unload-models`, `/api/keyring` (+ new tests - principle 15), cleanup scheduler, retention, file logs under `STORAGE_ROOT\Logs`, Windows service installer pair (Rust service wrapper registering `entity_in_the_shed` **alongside** the Python `entity-in-the-shed` per D9; the installer verifies port and storage-root isolation), cloudflared scripts ported | the app runs as a Windows service with no Python installed, installable while the old service is still registered, without a port or storage clash; reboot restores state; `current_process_account` reports the service identity |
| **9** | Full parity sweep: side-by-side harness runs every route against the Python app and the Rust app and diffs the JSON; unknown-key warning activated (§5); env re-count script re-run; guard catalogue complete; `docs/` current; **this plan deleted per principle 21**, deferred coding → `TODO/Loose-Ends.md` | every `docs/PARITY.md` row ticked or justified; 49 ported test modules + 8 JS suites green against the Rust server; the side-by-side diff is empty except §7 rows |

### 8.1 Staging rules

- **A crate is created and filled by the first stage with a production caller for it** (D4). `shed-json` fills at Stage 2 (model payloads, `models_cache.json`) and again at Stage 5 (tool arguments); `shed-text` at 2 (model-name normalisation), 4 (reply cleanup), 6 (markup/sound tags); `shed-store` at 4 (conversation files) and 7 (mind/client/outbox files).
- **`X-Request-ID` + the error envelope land at Stage 3**, the first stage serving `/api/*` — the source scopes both hooks to `/api/` paths (973–1027), so middleware before then would guard nothing.
- **`utils/env.py` helpers arrive with their first call site**; the staging list is a comment table in `shed-config/src/env.rs`. Three helpers are never ported: `getenv_first` (imported, called nowhere — re-verify at Stage 1), `encode_token_bytes`/`decode_token_bytes` (used only by the dropped IMAP tools), `initialize_mctrader_credentials` (`pass`).
- **Every dependency and feature is named for the code that uses it**; an unused `Cargo.toml` feature entry is principle 16 and each one carries a comment naming its consumer. `fancy-regex` (not `regex`) is pre-justified: `utils/text.py` uses look-around and a backreference (`(?s)(\*+).*?\1`) that RE2 cannot express — verify that claim against the file at Stage 2 before relying on it.

## 9. Test port map (49 pytest modules + `conftest.py` + 8 JS suites)

| Destination crate | Ported from | Stage |
|---|---|---|
| `shed-config` | `test_env_utils` | 1 |
| `shed-json` | `test_json_util_utils` | 2 |
| `shed-text` | `test_text_utils` | 2 → 4 → 6 (each part ports with its stage, §8.1) |
| `shed-store` | `test_entity_store_utils` | 4 |
| `shed-models` | `test_models_utils`, `test_lm_router_utils`, `test_agent_models` | 2 |
| `shed-llm` | `test_lm_streaming` | 2 |
| `shed-prompts` | `test_prompt_utils`, `test_prompt_cache` | 3 |
| `shed-characters` | `test_characters`, `test_character_profiles` | 3 |
| `shed-conversation` | `test_conversation_history_utils`, `test_conversation_housekeeping`, `test_conversation_identity` | 4 |
| `shed-chat` | `test_reply_utils`, `test_reply_recovery`, `test_reply_stream_scanner`, `test_reply_stream_publisher`, `test_sentence_splitter`, `test_glm4`, `test_glm4_followup` | 4 |
| `shed-tools` | `test_pending_trades`, `test_trade_confirmation_flow` | 5 |
| `shed-mcp` | `test_mcp_utils`, `test_mcp_args_utils`, `test_mcp_argument_guard` | 5 |
| `shed-tts` | `test_tts_utils`, `test_tts_cache`, `test_tts_failure_message`, `test_speech_pauses`, `test_voice_tones` | 6 |
| `shed-entity` | `test_entity_client`, `test_outbox`, `test_event_stream` | 7 |
| `shed-mind` | `test_entity_loop`, `test_mind_loop`, `test_mind_events`, `test_mind_goals`, `test_mind_state`, `test_mind_user`, `test_mind_events_wiring`, `test_mind_state_wiring`, `test_mind_user_wiring` | 7 |
| `shed-http` | `test_app_stylesheet`, `test_frontend_cache_headers` (3); `test_api_chat` (4); `test_test_speak_route` (6); `test_http_server` (1, then again at 4 and 7 — its waitress/Werkzeug assertions split across the stages that own the behaviour, §7.1) | 1→3→4→6→7 |
| `shed-server` | new: startup order, `--smoke` | 1, grown per stage |
| `shed-testutil` | `conftest.py` fixtures → temp storage roots, fake LM Studio, fake MCP server, fake Edge endpoint | 2, extended per need |
| `shed-gates` | no Python source: frozen-asset, layering, env-read, settings, guard-catalogue gates; tests live in each gate module | 0 (assets, layering, env-read), 1 (settings), guards with each guard |
| `tests/*.js` | the 8 browser suites **unchanged**, `npm test`; run against the Rust server from Stage 3 | 0 |

The three `_wiring` suites test `agent.py` wiring, not the util; they port as `shed-mind` integration tests driving the slow tick — exactly the wiring the Rust version must reproduce. `test_glm4*` are model-specific regressions graded against golden fixtures captured from the Python app at Stage 4, before it is retired (D8).

## 10. Risks and mitigations

| # | Risk | Mitigation |
|---|---|---|
| 1 | **Edge TTS v2 `Sec-MS-GEC` token in Rust** — the source's `requirements.txt` comment (read this session) states v1 tokens are refused with a 403 handshake the app cannot detect or repair | Stage 6 opens with a time-boxed spike. If it fails: Windows-SAPI-only, Edge recorded in `TODO/Loose-Ends.md`, failure-message contract kept so the user is told exactly what is missing |
| 2 | **Header semantics axum handles differently** (the `Connection` rule, hop-by-hop headers, SSE buffering) | `docs/CONTRACT.md` records the header rules; a contract test asserts the exact header set on every route, including the absence of `Connection` |
| 3 | **Concurrency-model change** (waitress/Werkzeug threads + global lock vs tokio tasks) | `TurnGate` is one owner with a test reproducing the busy payload under two simultaneous requests |
| 4 | **Heuristic drift** (reply usability, grounding, salience, recall scoring, tone tags) | golden fixtures captured from the Python app before retirement (D8); the JS suites run against the Rust server from Stage 3, not at the end |
| 5 | **JSON drift breaking the frozen 193 KB page / 119 KB helpers** | contract tests generated from the JS helpers' expectations (`buildOutboxRequestUrl`, `buildEntityStreamUrl`, `buildEntityStateUrl`, `readOutboxMessages`, `nextOutboxPollDelayMs`); a server-side change fails fast |
| 6 | **MCP maturity** (source needs `mcp>=1.27` over stdio) | `shed-mcp` isolates the protocol behind one client type; a fake MCP server in `shed-testutil` is the primary gate; the real mctrader server is an integration test only |
| 7 | **Windows service + keyring under a service account** (a LocalSystem service cannot read another account's credential store — the source README documents this trap) | Stage 8 runs the installer pair and asserts `current_process_account` reports the service identity; the installer keeps the `-ServiceAccount` escape hatch |
| 8 | **Scale** (11,607 lines; ~890-line character catalogue; ~332-line `run_agent`) | stage order follows the dependency direction (§3.3); no stage carries more than one monolith region |
| 9 | **Model-discovery drift** from LM Studio's local store layout | discovery tested against a captured directory fixture; the `models_cache.json` fallback ports so a discovery failure degrades like the Python app |
| 10 | **Dependency resolution offline** — some planned crates (`fancy-regex`, `keyring`, `tokio-tungstenite`, `rmcp`) may not be in the local cargo cache | Stage 0's `cargo add --dry-run` probe names any unresolvable pin **before** code is written against it; each stage resolves its new crates in front of the operator and records the resolved version in `Cargo.lock` |

## 11. Inherited-claims register

The previous attempt's claims, graded by this plan's own verification. **Confirmed** claims are safe to rely on; **corrected** rows carry this plan's number; **unverified** rows have a named check at a named stage.

| Claim | Grade | Evidence / check |
|---|---|---|
| `agent.py` = 11,607 lines; 24 pure utils; 110 character files; 7 prompt files; 49+8 test modules; 35 routes | **confirmed** | counted this session |
| Region map anchors (run_agent 7959, call_lm_engine 6808, synthesize_speech 3695, scanner 5592, SSE 9524, stream 11448, health 11503, run_http_server 11553, …) | **confirmed** | ~25 anchors grepped this session, all match |
| Werkzeug default (`USE_WAITRESS` false @580; `.env.example` ships `USE_WAITRESS=0`) | **confirmed** | read this session; source README contradicts — source-doc bug noted |
| Busy path = 200 with the exact reply text; deferred = 202; job three shapes; sentences re-attachment | **confirmed** | read 9389–9510 this session |
| SSE frame format, headers, refusal codes, registry semantics, single door | **confirmed** | read 9524–9644, 11448–11499 this session |
| Three template substitutions | **confirmed** | placeholders + render call grepped |
| Dropped routes have 0 client references; `test-speak` pinned by tests | **confirmed** | reference counts this session |
| edge-tts v1-token 403 rationale | **confirmed** | source `requirements.txt` comment, read this session |
| Distinct env names "150 / 146 in agent.py" | **corrected → 152 / 148** | own count (§2.5); difference is script-scope names (`SERVICE_NAME`) and `os.environ.get` sites |
| "31 of 35 routes ported" | **confirmed** | own route table + own zero-reference counts |
| dotenvy rejects Windows backslash paths (`LMSTUDIO_MODELS_DIR=D:\Models`), so a hand-written `python-dotenv` grammar reader is needed | **unverified (U1)** | Stage 1 spike against the operator's real `.env`; if the claim is false, use `dotenvy` and this row becomes history |
| `serde_json::Map` key order equals Flask's sorted `jsonify` bytes | **unverified (U2)** | Stage 1: capture live `/health` bytes from the running Python app; parity test compares bytes, not intent |
| hyper's `half_close` default makes a write-half-close smoke probe read nothing | **unverified (U3)** | check hyper's docs/source when `--smoke` is written (Stage 1); the probe design adapts either way |
| Cargo cache contents (which pins are already local) | **confirmed (U4 closed, Stage 0)** | `cargo add --dry-run` probe 2026-11-05: all 14 planned external crates resolve from this machine - axum, tokio, reqwest, tower, http-body-util, serde_json, futures-util, rmcp, tokio-tungstenite, keyring, fancy-regex, tracing, tracing-subscriber, windows v0.62.2. Note for Stage 6: the SAPI feature is `Media_SpeechSynthesis` (not `Media_Media_SpeechSynthesis`). Exact versions land in `Cargo.lock` as each stage adds its crates for real |
| `utils/text.py` needs look-around/backreference (→ `fancy-regex`) | **unverified (U5)** | re-read the regexes at Stage 2 before adding the dependency |
| `SERVICE_USERPROFILE` is read by the service wrapper | **unverified (U6)** | trace it when Stage 8 ports the service |

## 12. Definition of done

A stage is not complete until principle 20 holds: syntax validated, existing tests pass, new tests pass, dead code removed, call sites updated, clippy/fmt clean under `-D warnings`, temp and one-off script files deleted. The project is not complete until Stage 9's side-by-side sweep is empty except §7 rows, this plan is deleted, and any deferred coding item sits in `TODO/Loose-Ends.md` behind a `********************************` separator.
