# PLAN — Rust clone of Entity In The Shed

Source app: `D:\Entity-In-The-Shed` (Python 3.13 / Flask 3.0.7 + waitress — `agent.py` 11,607 lines,
730 top-level nodes + 24 pure `utils/` modules).
Target: `D:\Entity_In_The_Shed` (Rust workspace).
Rules: `How_To_Code.md` governs everything below. Principle 21 — this file is deleted when its
coding is finished; anything deferred moves to `TODO/Loose-Ends.md`.

Status: **planning only — no code written yet.** Every number in §2 was read out of the source; if a
future stage contradicts §2, §2 is wrong and gets corrected, not the other way round.

---

## 1. Locked decisions

| # | Decision | Consequence |
|---|---|---|
| D1 | Phased parity, core first | Stages 0–9 (§8). Each stage ends with `fmt + clippy -D warnings + test + parity gate` green before the next starts. |
| D2 | Reuse the browser client byte-for-byte | `templates/index.html`, `static/app-helpers.js`, `styles/app.css`, `styles/*.webp` are copied **unchanged** and frozen. The Rust server must satisfy the existing HTTP/SSE contract exactly. The 8 `tests/*.js` suites stay unchanged and are the client gate. |
| D3 | Native Rust TTS | Hand-written Edge read-aloud WebSocket client + a Rust implementation of the v2 `Sec-MS-GEC` token, plus `Windows.Media.SpeechSynthesis` via the `windows` crate. No Python at runtime. |
| D4 | Cargo workspace, one crate per domain | Library crates mirroring the 24 `utils/` modules plus the domain groups in `agent.py`. `agent.py` is decomposed, not transliterated. |
| D5 | Port the 49 pytest modules to Rust tests | Unit tests in each crate (`#[cfg(test)]` + `tests/`). HTTP tests drive the real router with `reqwest`. The 8 JS suites unchanged. |
| D6 | Improving logic is in scope | Where the Python is tangled or has a known defect, the Rust is written better — same observable behaviour, cleaner internals. Every deliberate divergence is recorded in §7. |
| D7 | No Python at runtime, no shelling out to Python | The Windows service, the TTS path and the MCP client are all native Rust. |

---

## 2. Source inventory
### 2.1 `agent.py` — 11,607 lines, 730 top-level nodes

Region map (line ranges are contiguous and cover the whole file; every name was read from the file).

| Lines | What is there |
|---|---|
| 1–41 | stdlib imports (`csv asyncio contextvars hashlib datetime email html ipaddress ast json logging mimetypes os queue random re shutil socket subprocess tempfile threading time collections functools pathlib string urllib uuid`), then `edge_tts`, `requests`, `dotenv`, `keyring`, `flask`, `waitress.serve`, `traceback`. Line 12 prints the startup banner. |
| 43–281 | 24 `from utils.…` import blocks (one per utils module, in the order listed in §2.2) |
| 283–296 | `COMMON_NON_TERMINAL_ABBREVIATIONS` |
| 301–354 | `app = Flask(...)` + storage constants: `STYLE_DIR STATIC_DIR STORAGE_ROOT MEMORY_DIR CHARACTER_OUTLINE_FILENAME LOG_DIR MIND_DIR DEFAULT_MIND_SLUG OUTBOX_DIR OUTBOX_CLIENTS_DIR MIND_USERS_DIR TTS_CACHE_DIR CHARACTER_PROFILES_DIR CHARACTER_FILE_SUFFIX MODEL_LOCAL_MAP_FILE LOCAL_MODEL_MAP` |
| 357–372 | `_load_local_model_map`, `_save_local_model_map` |
| 376–455 | LM Studio model + stop-token config (`AI_MODEL`, `AI_MODEL_LOCAL_OVERRIDE(S)`, `LM_ENGINE_HOST/NORMAL_MODEL/KEEP_ALIVE/NUM_PREDICT/NUM_CTX/LARGE_MODEL_CTX/LARGE_MODEL_SIZE_GB/MAX_GENERATION`, 4 per-mode temperatures, `LM_ENGINE_STOP_TOKENS`, `_DEFAULT_LM_ENGINE_STOP_TOKENS`, `merge_stop_tokens`, `AI_MODEL_DISCOVERY_TIMEOUT`) |
| 458–526 | timeouts + mctrader config (`OPENAI_API_KEY/BASE/REQUEST_TIMEOUT`, `LM_ENGINE_REPLY_TIMEOUT`, `MCTRADER_BASE_URL/API_TOKEN` — read, never used elsewhere, `MCTRADER_LOGIN_KEYRING_*`, `MCTRADER_TIMEOUT`, `MCTRADER_MCP_{TRANSPORT,COMMAND,ARGS,ENV,URL,TIMEOUT,CACHE_TTL}`, `MCTRADER_AUTH_REQUIRED_TOOL_ERROR`, `MCTRADER_SIGNIN_REPLY`, `LM_ENGINE_TRANSIENT_RETRIES`, `LM_ENGINE_STREAM_CHAT`, `LM_ENGINE_PROMPT_CACHE*`, `LM_ENGINE_STREAM_IDLE_TIMEOUT`, `LM_ENGINE_RETRY_{BASE,JITTER}_MS`, `LM_ENGINE_MODEL_ADMIN_TIMEOUT`, `LM_ENGINE_RESIDENCY_PROBE_TIMEOUT`) |
| 548–555 | sampler penalties: `LM_ENGINE_{REPEAT,PRESENCE,FREQUENCY}_PENALTY`, `REPEAT_LAST_N`, `DRY_*` |
| 556–596 | TTS + server + web-search config (`TTS_TIMEOUT`, `TTS_PROVIDER` default `"windows"`, `TTS_DEFAULT_VOICE`, `TTS_DEFAULT_RATE`, `TTS_BROWSER_FALLBACK`, `TTS_CACHE_*`, `APP_HOST`, `APP_PORT` 5005, `USE_WAITRESS`, `WAITRESS_THREADS`, `WAITRESS_CHANNEL_TIMEOUT`, `WEB_SEARCH_ENABLED/TIMEOUT`, `WEB_FETCH_MAX_CHARS`, `MODEL_SELECTION_REQUIRES_PASSWORD`, `DEFAULT_RESTRICTED_MODEL`, `MODEL_FALLBACK_TIMEOUT_SECONDS`) |
| 598–702 | `BASE_SYSTEM_PROMPT`, `PROMPT_TEMPLATE_PATHS`, `DELIVERY_TTS_DEFAULTS`, `TONE_TTS_SETTINGS` (46 lines of tone→voice settings), `TOOL_DESCRIPTIONS`, `BROKER_NUMBER_WORDS` |
| 704–737 | conversation + browser-chat job state (`RECENT_EVENT_IDS`, `CONVERSATION_{MAX_MESSAGES,REUNION_SECONDS,IDLE_SECONDS,HOUSEKEEPING_POLL_SECONDS,HOUSEKEEPING_COOLDOWN_SECONDS}`, `DETAILED_MEMORY_DAYS`, `SUMMARY_MEMORY_DAYS`, `GENERATED_FILE_RETENTION_DAYS`, `CONVERSATION_FILE_RETENTION_DAYS`, `SERVICE_LOG_RETENTION_DAYS`, `CLEANUP_INTERVAL_SECONDS`, locks, `ACTIVE_CHAT_REQUESTS`, `BROWSER_CHAT_{JOB_TTL_SECONDS,MAX_SENTENCES,PENDING_STATUS,JOBS}`) |
| 742–777 | outbox + SSE config (`ENTITY_PROACTIVE_ENABLED`, `ENTITY_SAY_ENABLED`, `ENTITY_OUTBOX_{MAX_MESSAGES,TTL_SECONDS}`, `ENTITY_UNSEEN_DELIVERY_TTL_SECONDS`, `ENTITY_OUTBOX_POLL_MS`, `OUTBOX = OutboxStore(...)`, `ENTITY_SSE_{ENABLED,HEARTBEAT_SECONDS,MAX_SUBSCRIBERS,MAX_LIFETIME_SECONDS,RECONNECT_MIN_SECONDS,QUEUE_MAX}`, `SSE_SUBSCRIBERS`, `SSE_LAST_CONNECT_AT`, `LAST_TURN_ENDED_AT`) |
| 782–840 | slow-loop config (`ENTITY_SLOW_LOOP_ENABLED`, `ENTITY_DRY_RUN`, `ENTITY_TICK_INTERVAL_{MIN,MAX}_SECONDS`, `ENTITY_IDLE_GRACE_SECONDS`, `ENTITY_QUIET_HOURS`, `ENTITY_CLIENT_STALE_SECONDS`, `ENTITY_SPEECH_BUDGET`, `ENTITY_MIN_UTTERANCE_GAP_SECONDS`, `ENTITY_MAX_UTTERANCES_{PER_HOUR,PER_DAY}`, `ENTITY_MAX_IGNORED_STREAK`, `ENTITY_BACKGROUND_NUM_PREDICT`, `ENTITY_THOUGHT_JOURNAL_LIMIT`, `ENTITY_REPEAT_GUARD_LINES`, `MIND_STATE_THOUGHT_PREVIEW`, `BACKGROUND_LLM_LOCK`, `MIND_LOOP_{STOP_EVENT,THREAD,STATE}`) |
| 847–955 | user/fact/event/goal/mind-state config + caches (`ENTITY_USER_MEMORY_ENABLED`, `ENTITY_FACT_LOG_LIMIT`, `ENTITY_{EVENTS,GOALS,PLANNER}_ENABLED`, `ENTITY_GOALS_LIMIT`, `ENTITY_EVENT_{QUEUE_LIMIT,MAX_AGE_SECONDS}`, `ENTITY_MARKET_MOVE_PERCENT`, `ENTITY_BACKGROUND_TOOL_ALLOWLIST`, `ENTITY_INACTIVITY_SECONDS`, `ENTITY_MIND_STATE_ENABLED`, `ENTITY_CURIOSITY_HALF_LIFE_SECONDS`, `ENTITY_URGENCY_HALF_LIFE_SECONDS`, `ENTITY_FULL_SILENCE_SECONDS`, `ENTITY_FULL_URGENCY_MOVE_PERCENT`, `ENTITY_MIND_ENERGY_FLOOR`, `ENTITY_STATE_TEMPERATURE_*`, `VOICE_LIST_CACHE`, `COUNTRY_INDEX_CACHE`, `COUNTRY_FILE_CACHE`, `PROMPT_TEMPLATE_CACHE`, `MCTRADER_MCP_CACHE`) |
| 956–970 | `mcp_token_var`, `request_id_var`, `chat_job_id_var`, `background_turn_var` (contextvars) |
| 973–1035 | HTTP plumbing: `current_request_id`, `api_error_response`, `api_json_boundary`, `assign_request_id` (`@app.before_request`), `attach_request_id_header` (`@app.after_request`), `_is_retryable_lm_failure` |
| 1038–1236 | LM HTTP transport + retry + streaming: `call_lm_http_with_retry`, `LmStreamUnavailable`, `read_lm_chat_stream`, `call_lm_http_stream_chat`, `_stream_rejection`, `_lm_stream_backoff`, `lm_body_indicates_model_unloaded` |
| 1239–1629 | model discovery: `KNOWN_MODELS MODEL_PATHS MODEL_METADATA`, `_apply_known_models`, `_load_cached_model_discovery`, `_persist_model_discovery_cache`, `lmstudio_models_root`, `discover_lm_studio_models_from_disk`, `ingest_discovery_item` (174 lines), `discover_lm_studio_models` |
| 1631–1724 | `NAME_SEX_HINTS`, `powershell_command`, `fetch_AI_MODEL_names` |
| 1727–2103 | model/background resolution + MCP tool layer: `resolve_AI_MODEL`, `resolve_background_model`, `resolve_mctrader_mcp_transport`, `build_mctrader_mcp_config`, `validate_mctrader_mcp_config`, `build_mcp_tools_response`, `run_mctrader_mcp_session` (async), `list_mctrader_mcp_tools`, `get_mctrader_mcp_status`, `mcp_tool_shadows_local_tool`, `resolve_tool_descriptions`, IP-token cache, `call_mctrader_mcp_tool`, `build_mcp_tool_wrapper`, `resolve_available_tools` |
| 2106–2193 | prompt templates + tone/delivery TTS mapping + browser model options |
| 2196–3085 | **character catalogue** (sex normalisation, pronouns, country files, roster building, label disambiguation, fact sheets, profiles, outline, character system prompt, prompt-cache identity) — 890 lines, the largest domain in the file |
| 3092–3276 | voice catalogue + selection (`VOICE_TIER_*`, `voice_tier`, `accent_preference`, `voice_preference_score`, `list_windows_voices`, `list_edge_voices`, `list_available_voices`, `choose_tts_voice`, `normalize_voice_depth`) |
| 3279–3358 | laugh/fart asset selection (`should_include_reply_laugh`) |
| 3361–3473 | retention/cleanup + `cleanup_scheduler_loop` |
| 3476–3782 | TTS synthesis: `edge_rate_string`, `edge_pitch_string`, `edge_volume_string`, `edge_tts_version`, `synthesize_speech_with_edge`, `_edge_output_has_audio`, `synthesize_speech_with_windows`, `build_tts_audio_cache`, `speech_cache_key`, `synthesize_speech` |
| 3785–4174 | conversation records: path/identity/claim-legacy, load/save, delivery style, `summarize_conversation_messages`, `update_conversation_user_details`, `get_conversation_history` |
| 4185–4526 | housekeeping + profile workers (`load_conversation_record_strict`, `update_conversation_profile`, `profile_conversation_worker`, `start_conversation_profile_update`, `wait_for_conversation_profile_workers`, `housekeep_conversation_record_file`, `run_conversation_housekeeping`, `conversation_housekeeping_loop`) |
| 4529–5000 | recall + persona guardrails (`ORDINAL_LOOKUP`, `RECALL_STOPWORDS`, `SEMANTIC_TOKEN_MAP`, `score_recall_similarity`, `classify_character_question`, `answer_direct_character_{identity,origin}_question`, `enforce_persona_reply`, `try_answer_conversation_recall_question`, `append_conversation_message`) |
| 5003–5073 | template/path rendering + `run_configured_command` |
| 5076–5225 | IMAP email (`decode_mime_header`, `extract_email_body`, `email_connection`, `list_email_messages`, `fetch_email_by_uid`) — **not ported**, §11 decision 1 |
| 5228–5501 | local tools + the `TOOLS` dict (`tool_current_datetime`, DuckDuckGo `search_public_web`, `fetch_public_webpage`, `is_public_web_url`, mctrader start/stop/status tools, email tools, `tool_create_spreadsheet`) |
| 5504–6031 | reply text shaping + **streaming**: `normalize_reply_text`, `markdown_to_ssml`, `JsonReplyStreamScanner` (208 lines), `SentenceSplitter` (121 lines), `split_reply_after_stream`, `ReplyStreamPublisher` |
| 6037–6202 | structured reply documents (`REPLY_DOCUMENT_KEYS`, `_parse_structured_candidate`, `_parse_model_reply_document`, `settle_prose_tool_calls`, `parse_model_json_content`) |
| 6207–6497 | router admin + residency (`fetch_router_model_listing`, `warn_if_model_swap_needed`, `request_router_model_unload`, `unload_all_models`, `reset_gpu_and_models`, `find_local_lmstudio_model_match`, `resolve_model_context_window`, display-name formatting) |
| 6500–7131 | the completion call: `call_openai_api`, `_call_lm_engine_streaming`, prompt-block store (`probe_prompt_block_store` … `prompt_cache_call_kwargs`), `parse_chat_completion_response`, `call_lm_engine` (324 lines) |
| 7134–7301 | timing + system-prompt assembly (`record_timing`, `format_timing_summary`, `build_runtime_context_prompt`, `build_voice_mode_system_prompt`, `build_combined_system_prompt`), `should_force_web_search`, `extract_broker_id_from_text` |
| 7304–7757 | mctrader tool-call inference (argument filling from free text, screener tool selection, continuation/follow-up resolution, forced tool-call merge, `resolve_model_temperature`, `resolve_reply_segments`) |
| 7760–7956 | persistence + tool execution + recovery (`persist_exchange_reply`, `build_protected_tool_auth_reply`, `build_forced_web_search_tool_calls`, `execute_mctrader_tool_calls`, `recover_agent_reply`, `build_agent_follow_up_messages`) |
| 7959–8417 | `run_agent` (332 lines), `ensure_complete_reply`, `ensure_usable_reply`, `is_duplicate_event` |
| 8418-9510 | **routes** - pages/assets, catalogue, models, TTS, MCP/keyring, restart, browser-chat job machinery (see `docs/PARITY.md`) |
| 9524–9644 | SSE machinery (`SseSubscriber`, register/unregister, `publish_to_client`, `speak_to_client`, `format_sse_frame`) |
| 9647–10493 | mind store + mind reasoning (thoughts, user model, facts, goals, events, mind state, planner, `browser_chat_foreground_busy`) |
| 10496–11050 | the slow tick (`iter_entity_client_records`, `latest_entity_client`, `build_background_thought_prompt`, `compose_background_thought`, `entity_utterance_state`, `slow_tick_inputs`, `run_entity_slow_tick`, `mind_loop_worker`, `start_mind_loop`, `stop_mind_loop`) |
| 11053–11499 | entity routes + SSE response construction (`outbox_message_payload`, outbox/ack/events/say/proactive/state, `sse_catchup_frames`, `sse_live_frames`, `entity_stream_response`, `api_entity_stream`) |
| 11502–11607 | `healthcheck`, `initialize_mctrader_credentials` (a `pass` no-op), `configure_logging`, `run_http_server`, `api_refresh_models`, `if __name__ == "__main__": run_http_server()` |

Env reads in `agent.py`, exact: 62 `os.getenv(`, 66 `getenv_int(`, 17 `getenv_float(`, 2 `getenv_timeout_seconds(`, 28 `parse_bool(`, 2 `parse_json_env(` — **146 distinct env names** in `agent.py` alone, **150** across `agent.py` + `utils/` + `scripts/` (`utils/` adds `MCTRADER_DEFAULT_EXCHANGE_ID`, `MCTRADER_MCP_SERVER_NAME`, `USER`, `USERNAME`). Counted by matching a literal env-name string as the first argument of those helpers.

### 2.2 `utils/` — 24 modules, all pure (none import Flask or `agent`)

| Module | Lines | Rust crate | What it holds |
|---|---|---|---|
| `env.py` | 100 | `shed-config` | `BASE_DIR`, `resolve_storage_dir`, `getenv_int/float/timeout_seconds`, `parse_bool`, `getenv_first`, `normalize_AI_MODEL_name`, `current_process_account`, `parse_json_env` |
| `json_util.py` | 66 | `shed-json` | `make_json_safe`, token-byte encode/decode, `parse_json_response_strict` |
| `text.py` | 552 | `shed-text` | `estimate_tokens`, tone-tag stripping, `normalize_reply_markup`, vocalisation respelling (`mmm`/`ahhh`), `segment_text_by_brackets`, `merge_main_segments_with_punctuation`, `spoken_text_from_segments` |
| `characters.py` | 102 | `shed-characters` | character id/cache key construction |
| `models.py` | 170 | `shed-models` | size parsing, browser model/language normalisation, discovery candidate parsing + sorting, embedding-model filtering |
| `lm_router.py` | 136 | `shed-models` | `RouterModel`, unload payloads, residency mismatch reporting |
| `prompt.py` | 164 | `shed-prompts` | `parse_prompt_markdown`, template variables, language/tone system prompts |
| `prompt_cache.py` | 154 | `shed-prompts` | LM Studio prompt-block naming/expansion, `PromptCachePlan` |
| `conversation_history.py` | 445 | `shed-conversation` | record/message/summary/compaction types, client identity, filename rules, `plan_conversation_compaction` |
| `entity_store.py` | 259 | `shed-store` | `atomic_write_json`, `atomic_write_lines`, `read_jsonl_tail`, `append_jsonl`, `load_json_with_recovery` + quarantine, client-id minting |
| `entity_client.py` | 127 | `shed-entity` | `ClientRecord`, `merge_client_record`, `advance_utterance_counters` |
| `mind_outbox.py` | 363 | `shed-entity` | `OutboxStore` (queued/delivered/acked/dropped/expired, delivered-id ring) |
| `entity_loop.py` | 548 | `shed-mind` | 20 veto codes, `GroundingReport`, `SpeakContext`, `ThoughtRecord`, `TickOutcome`, `SlowTickDeps`, `compute_speech_budget`, `compute_slow_loop_decision`, `should_speak`, `run_slow_tick` |
| `mind_events.py` | 483 | `shed-mind` | event kinds/normalisation, grounding checks, salience prefilter, dedupe, pruning |
| `mind_goals.py` | 361 | `shed-mind` | `Goal`, provenance, cooldowns, planner output → goals |
| `mind_state.py` | 453 | `shed-mind` | curiosity/urgency/energy drives, half-lives, directives, `temperature_delta`, `build_mind_state_prompt` |
| `mind_user.py` | 567 | `shed-mind` | user model extraction (name/interests/avoidances/work/topics), familiarity, style resolution, fact records |
| `mcp.py` | 461 | `shed-mcp` | tool normalisation, auth gating, credential-refusal detection, redaction, call tracing, token extraction |
| `mcp_args.py` | 273 | `shed-mcp` | free-text → tool argument extraction (symbol, exchange, position type, limit, rank_by, dry_run) |
| `tts.py` | 184 | `shed-tts` | delivery/tone TTS settings, MP3/WAV sniffing, `describe_tts_failure`, `audio_payload_has_sound` |
| `tts_cache.py` | 506 | `shed-tts` | `TtsAudioCache`: keyed cache, single-flight, staging files, size/entry/byte caps, delete grace |
| `speech_pauses.py` | 291 | `shed-tts` | **MP3 frame reader**, punctuation pause windows, `speech_pause_cuts`, `trim_edge_pauses` |
| `reply.py` | 389 | `shed-chat` | completeness/usability heuristics, tool-call envelope stripping, segment extraction, teaser detection |
| `pending_trades.py` | 311 | `shed-tools` | trade preview store + confirmation code state machine |

Cross-imports inside `utils/` (6): `json_util→text`, `mcp→env`, `mcp→json_util`, `conversation_history→entity_store`, `mind_outbox→entity_store`, `prompt→models`. Only `pending_trades.py` has module-level mutable state.

### 2.3 Assets, data and tests

- Frozen client: `templates/index.html` (194 KB), `static/app-helpers.js` (120 KB), `styles/app.css` (12 KB), `styles/information_info_1565.webp` (6 KB).
- Data read at runtime: `characters/*_characters.json` (110 entries), `voices.json` (136 KB), `prompts/*.md` (7 files), `outputs/tts/{laugh,fart}/` audio.
- `last_model_index.txt` is in the repo root in the Python app — the clone moves it under `STORAGE_ROOT` (logged as a divergence).
- Tests: 49 `tests/test_*.py` + `conftest.py`, and 8 `tests/*.js` browser suites. `tests/__pycache__` still holds `.pyc` for four `test_eodhd_screener_*` files whose sources were deleted — nothing to port.
- `scripts/`: Windows service installers (`install-entity-in-the-shed-service.ps1`, `entity-in-the-shed-service.py`), cloudflared tunnel service, `cdp_mobile_probe.py` (drives the JS suites), `check_voices.py`, `verify_refactor.py`.
- Python deps → Rust: Flask+waitress → `axum`+`tokio`; `requests` → `reqwest`; `python-dotenv` → `dotenvy`; `mcp` → `rmcp` (or a hand-written JSON-RPC stdio client); `edge-tts` → hand-written WebSocket client (`tokio-tungstenite`); `pywin32` → `windows` crate; `keyring` → `keyring` crate.

### 2.4 Runtime shape

`if __name__ == "__main__": run_http_server()`; `run_http_server` (11553–11592) runs, in order: `configure_logging` → `initialize_mctrader_credentials` (no-op `pass`) → `discover_lm_studio_models` → `start_cleanup_scheduler` → `start_conversation_housekeeper` → `start_character_catalog_warmer` → `start_mind_loop` → the server.

The server branch is the opposite of what a reader expects: `USE_WAITRESS` defaults to **false**
(`agent.py:580`) and `.env.example` ships `USE_WAITRESS=0`, so the app normally runs on
**Werkzeug threaded** (`app.run(host, port, debug=False, threaded=True)`, 11592) and `waitress.serve(...)`
(11583–11590) is the opt-in. The source's own comment says waitress is "kept as a branch rather than a
replacement so the change can be backed out by unsetting one variable". The clone has one server
(tokio + axum) sized like the waitress branch — bounded workers, a channel/read timeout — because that is
the configuration a long-lived SSE stream actually needs.

Daemon threads: `cleanup-scheduler`, conversation housekeeper, character warmer, `mind-loop`,
per-request browser-chat jobs, conversation-profile workers.

Everything that makes the entity act unprompted is **off by default**, in the code and in
`.env.example`: `ENTITY_PROACTIVE_ENABLED=false` (master opt-in, `agent.py:742`),
`ENTITY_SLOW_LOOP_ENABLED=false` (782), `ENTITY_DRY_RUN=true` (785), `ENTITY_SAY_ENABLED=false` (746),
`ENTITY_SSE_ENABLED=false` (760). `/api/entity/state` publishes those flags (`proactive_available`,
`sse_enabled`, `say_enabled`) and the client stands its stream up or down accordingly. The clone keeps the
same defaults and the same opt-in chain; Stage 7's exit gate runs with them deliberately switched on.

---

## 3. Target architecture

### 3.1 Workspace

```
Entity_In_The_Shed/
├── Cargo.toml                [workspace] + [workspace.dependencies] + [workspace.lints]
├── rustfmt.toml              max_width = 100, stable-channel options only
├── clippy.toml               too-many-lines-threshold = 50 (principle 5)
├── package.json              npm test → node --test "tests/*.js"
├── .env.example              re-authored under principle 24 (Stage 1)
├── .gitignore
├── README.md
├── docs/
│   ├── ARCHITECTURE.md       crate map + dependency direction + ownership (§3.2, §3.3)
│   ├── CONTRACT.md           the frozen HTTP/SSE contract (§4)
│   ├── PARITY.md             the parity table, kept current (moved from TODO/)
│   └── GUARD_CATALOGUE.md    principle 23 index
├── templates/index.html      frozen client and data, copied byte-for-byte (decision D2).
├── static/app-helpers.js     The layout mirrors the Python app on purpose, so the copied
├── styles/                   files and the 8 browser suites need no edits at all: the
├── characters/, prompts/     suites do `require("../static/app-helpers.js")`.
├── outputs/tts/{laugh,fart}/
├── voices.json
├── crates/                   19 library crates (§3.2)
├── bins/shed-server/         the one binary
└── tests/
    ├── *.js                  the 8 browser suites, unchanged
    └── assets.manifest.json  path + bytes + sha256 of every frozen file
```

### 3.2 Crates (19 libraries + 1 binary)

| Crate | Source of its logic | Responsibility |
|---|---|---|
| `shed-config` | `utils/env.py` + `agent.py:376–955` | Typed `AppConfig`: every env name parsed once, validated, immutable. No other crate reads env. |
| `shed-json` | `utils/json_util.py` | JSON safety, strict response parsing, token-byte helpers |
| `shed-text` | `utils/text.py` | token estimation, tone-tag/markup normalisation, vocalisation respelling, bracket segmentation |
| `shed-store` | `utils/entity_store.py` | atomic JSON/JSONL writes, quarantine + recovery, client-id mint/normalise |
| `shed-logging` | `agent.py:11524–11550` + log conventions | one logger, one format, one destination |
| `shed-models` | `utils/models.py`, `utils/lm_router.py`, `agent.py:1239–1724`, `6207–6497` | model discovery + cache, discovery-from-disk, context windows, display names, router unload/residency |
| `shed-llm` | `agent.py:1038–1236`, `6500–7131` | LM Studio/OpenAI-compatible transport: retry, streaming, sampler payload, prompt-block store, response parsing |
| `shed-prompts` | `utils/prompt.py`, `utils/prompt_cache.py`, `agent.py:598–702`, `2106–2193`, `7134–7247` | prompt templates, tone/delivery maps, combined system prompt assembly, prompt-cache plan |
| `shed-characters` | `utils/characters.py`, `agent.py:2196–3085` | country/character catalogue, profiles, outlines, sex/pronoun handling, roster disambiguation |
| `shed-conversation` | `utils/conversation_history.py`, `agent.py:3785–5000` | conversation records, identity/filenames, compaction, summarisation, housekeeping, recall + persona guardrails |
| `shed-chat` | `utils/reply.py`, `agent.py:5504–6202`, `7760–8417` | reply shaping, JSON reply scanner, sentence splitter, stream publisher, `run_agent`, completeness/usability recovery |
| `shed-tools` | `agent.py:5003–5073` (mctrader argument/command templating), `5228–5501` (local tools), `utils/pending_trades.py` | local tool registry + implementations, spreadsheet writer, DuckDuckGo search/fetch, configured start/stop/status commands, trade preview + confirmation state machine. The IMAP block (`5076–5225`) is **not** ported (§11 decision 1) |
| `shed-mcp` | `utils/mcp.py`, `utils/mcp_args.py`, `agent.py:1727–2103`, `7304–7757` | MCP session/transport, tool normalisation, auth gating, redaction, argument inference, mctrader tool-call resolution |
| `shed-tts` | `utils/{tts,tts_cache,speech_pauses}.py`, `agent.py:3092–3276`, `3476–3782` | voice catalogue + selection, Edge WebSocket synthesis, Windows SAPI synthesis, audio cache, MP3 pause trimming, failure description |
| `shed-entity` | `utils/{entity_client,mind_outbox}.py`, `agent.py:9524–9644`, `10496–10602`, `11053–11107` | client records, outbox store, SSE subscriber registry, frame formatting, utterance history |
| `shed-mind` | `utils/{entity_loop,mind_events,mind_goals,mind_state,mind_user}.py`, `agent.py:9647–10493`, `10796–11050` | slow loop, vetoes, grounding, events, goals, drives, user model, thought journal |
| `shed-http` | `agent.py:8418–9510`, `11053–11607` | axum router, 31 of the source's 35 routes (§7.1 drops four), middleware, error envelope, static assets, deferred chat jobs |
| `shed-testutil` | — | dev-dependency only: temp storage roots, fake LM/MCP/Edge servers, fixture builders |
| `shed-gates` | — | dev-dependency only: the workspace-wide invariant gates (frozen assets, crate layering, the env-read rule), because no single domain crate owns them. Its tests live in each gate module (§9) |
| `shed-server` (bin) | `agent.py:11553–11592` | startup order, background tasks, graceful shutdown |

### 3.3 Dependency direction (must never be violated)

```
shed-config        → (nothing)
shed-json          → shed-config
shed-text          → shed-config
shed-logging       → shed-config
shed-store         → shed-config, shed-json
shed-models        → shed-config, shed-json, shed-text, shed-store
shed-prompts       → shed-config, shed-json, shed-text, shed-store
shed-characters    → shed-config, shed-json, shed-text, shed-store
shed-llm           → shed-config, shed-json, shed-text, shed-models, shed-prompts
shed-conversation  → shed-config, shed-json, shed-text, shed-store, shed-models, shed-prompts
shed-tools         → shed-config, shed-json, shed-text, shed-store
shed-mcp           → shed-config, shed-json, shed-text, shed-store
shed-chat          → shed-config, shed-json, shed-text, shed-models, shed-llm, shed-prompts,
                     shed-tools, shed-mcp, shed-conversation
shed-tts           → shed-config, shed-text, shed-store
shed-entity        → shed-config, shed-json, shed-store
shed-mind          → shed-entity, shed-models, shed-llm, shed-prompts, shed-conversation, shed-tools
shed-http          → the 16 crates above (it is the composition root, apart from the binary)
shed-server (bin)  → everything, including shed-http
shed-testutil      → the 16 crates above (dev-dependency only)
shed-gates         → shed-config, shed-testutil (dev-dependency only)
```

This table is `LAYERS` in `crates/shed-gates/src/layers.rs`, and the layering test reads each package's
`Cargo.toml` against it. No cycles; a crate may not depend on a crate that composes it;
`shed-testutil` and `shed-gates` are dev-dependency only and no shipped crate may depend on them.

---

## 4. The frozen contract (verified from the source)

Everything here is what `index.html` + `app-helpers.js` already expect. The Rust server reproduces it
byte-for-byte; a change here is a client break, so §4 is copied to `docs/CONTRACT.md` and treated as
the spec.

### 4.1 Cross-cutting

- `X-Request-ID`: assigned in `before_request` (`agent.py:1015–1020`), echoed by `after_request`
  (`1023–1027`) and again on every error response. Rust: one axum middleware layer on every route.
- Error envelope (`api_error_response`, `987–999`): `{"ok": false, "error": str, "code": str,
  "request_id": str}` with the status the handler chose; `api_json_boundary` (`1002–1012`) turns any
  unhandled handler exception into `500` / `code: "internal_error"` after logging the traceback with
  the request id. Rust: a `Result`-returning handler wrapper; **no panics reach the client**.
- Every `/api/*` handler is boundary-wrapped **except** `/api/entity/stream` and `/_routes` — the
  source explains why (`11474–11481`): a JSON boundary cannot catch a generator error after the
  response object is already returned. The Rust equivalent keeps the same rule: SSE errors are
  handled inside the stream, not by the envelope.
- Static/page responses carry `Cache-Control: no-store, no-cache, max-age=0, must-revalidate`,
  `Pragma: no-cache`, `Expires: 0` (`index` 8427–8429, `app.css` 8440–8442, and the same on
  `app-helpers.js` and the webp).
- `index.html` is rendered with exactly three substitutions: `chat_request_timeout_ms`
  (`LM_ENGINE_REPLY_TIMEOUT * 1000`, or `None`), `model_selection_requires_password`
  (`MODEL_SELECTION_REQUIRES_PASSWORD`), `browser_tts_enabled` (`bool(TTS_BROWSER_FALLBACK)`).
- Chat timing headers: `X-Entity-In-The-Shed-Timing` (truncated to 1000 chars) and
  `X-Entity-In-The-Shed-Total-Ms` (`f"{total_ms:.1f}"`).

### 4.2 `POST /api/chat` (9443–9510)

- Body: JSON; `deferred` is parsed with `parse_bool(..., default=False)`; a missing/empty `message`
  is a `400` `code: "message_required"`.
- Busy path — when `AI_CONVERSATION_LOCK` cannot be taken non-blockingly: **HTTP 200** with
  `{"ok": true, "reply": "The Entity is in consultation with another, please try later."}`. Not 429.
- Deferred path: **HTTP 202** `{"ok": true, "pending": true, "job_id": <32-hex>, "status": "pending"}`;
  the job is registered with `expires_at = now + BROWSER_CHAT_JOB_TTL_SECONDS` and `sentences: []`,
  and runs on a worker. Slot release stamps `LAST_TURN_ENDED_AT` (9063–9065) — the slow loop reads it.
- `GET /api/chat/jobs/<job_id>` (9389–9440) has three shapes and all three are the contract:
  - unknown/expired id → `404` `code: "chat_job_not_found"`;
  - `status == "pending"` → `{ok, pending: true, job_id, status, delivery_style, sentences[],
    sentence_count}`, where `delivery_style` is `""` until the worker has resolved the turn's style — the
    page voices streamed sentences from it, so an empty value is meaningful, not missing;
  - `status == "complete"` → the stored `response_payload` **with `sentences` re-attached** plus the chat
    timing headers, and a stored failure replays the recorded `http_status` / `error_code` / message.
  `sentences` is re-attached on the finished payload on purpose: the turn publishes its closing sentence
  after the last poll that could have carried it, so the completed payload is the only place the last
  sentence can still arrive from. The source's own comment says dropping it yields "a reply that is printed
  in full and spoken short of its ending". The Rust port keeps the re-attachment and the client-side
  de-duplication that goes with it.
- The single-slot rule (`ACTIVE_CHAT_REQUESTS` + `AI_CONVERSATION_LOCK`) is the app's concurrency
  contract: one conversation turn at a time, process-wide.

### 4.3 `POST /api/speak` (8861–8941)

- Request fields: `text`, `voice`, `provider`, `character_id`, `preferred_language`,
  `delivery_style`, `tone`. **`rate` and `pitch` are not read** — they come from
  `get_delivery_tts_settings(delivery_style)` then `apply_tone_tts_settings(..., tone)`.
- Success: the audio file with its mimetype, plus `X-TTS-Voice`, `X-TTS-Provider`,
  `X-TTS-Cache: hit|miss`, `Cache-Control: no-store`.
- Cache hits are served from the durable cache and **not deleted**; misses are written to a temp
  folder that is deleted (file then empty parent dir) ~1 s after serving.
- Failure: `api_error_response(describe_tts_failure(exc, edge_tts_version()), 500, "tts_failed")` —
  the message text is part of the contract because the client displays it.

### 4.4 Entity outbox + SSE

- `outbox_message_payload` (11053–11075) is the only view of a queued message: `seq`, `delivery_id`,
  `kind`, `trigger`, `text` (markup-normalised for reading), `text_markup` (the raw text, only when
  it differs), `character_slug`, `delivery_style`, `created_at`. Provenance stays server-side.
- `GET /api/entity/outbox?client_id=&limit=` → `{ok, client_id, messages[], count}`; it **drains**
  and a message stays deliverable until acked.
- `POST /api/entity/outbox/<delivery_id>/ack` → `{ok, delivery_id, changed}`; idempotent.
- `GET /api/entity/stream` (11474–11499): refusals are `404 entity_stream_disabled`,
  `429 entity_reconnect_too_fast`, `503 entity_stream_full`. Cursor comes from `?since=` or the
  leading integer of `Last-Event-ID` (ids are `<seq>-<ms>`).
- Frame format (`format_sse_frame` 9629–9640): optional `id:`, then `event:`, then one `data:` line
  **per line of the JSON payload** (multi-line payloads are split), terminated by a blank line.
- Stream body (`entity_stream_response` 11448–11470): first frame `event: state` with
  `{"client_id": ..., "ok": true}`, then catch-up `utterance` frames, then live frames with `": hb\n\n"`
  comment heartbeats every `ENTITY_SSE_HEARTBEAT_SECONDS`, ending at `ENTITY_SSE_MAX_LIFETIME_SECONDS`.
  Headers: `Cache-Control: no-cache`, `X-Accel-Buffering: no`, and **`Connection` must not be set** —
  waitress rejects it from a WSGI app (documented at 11458–11463). Rust must likewise not set it.
- `speak_to_client` (9600–9626) is the single door: every spoken line is queued **and** pushed. The
  docstring explains why both halves are mandatory; the Rust port keeps one door for the same reason.

### 4.5 Client invariants that pin the server

`index.html` and `app-helpers.js` are not edited, so: the deferred-chat flow (202 + job polling),
sentence-by-speech streaming from `sentences[]`, the busy reply text, the outbox poll standing down
while the SSE stream is up, `client_id` minting/echoing, and the three template substitutions are all
load-bearing. Any Rust-side "improvement" that changes one of these is out of scope for the clone and
goes to `TODO/Loose-Ends.md`.

---

## 5. Configuration

- The source reads **150 distinct env names** across `agent.py` + `utils/` + `scripts/` — 146 of them in
  `agent.py` alone, read through 62 `os.getenv` + 66 `getenv_int` + 17 `getenv_float` + 2
  `getenv_timeout_seconds` + 28 `parse_bool` + 2 `parse_json_env` calls. `.env.example` documents only **113**
  of them: 36 names the
  code reads are undocumented (`EMAIL_*`, `ENTITY_MAX_UTTERANCES_PER_{HOUR,DAY}`,
  `CONVERSATION_FILE_RETENTION_DAYS`, `LM_ENGINE_REPLY_TIMEOUT`, `MCTRADER_DEFAULT_*`, `LOG_LEVEL`,
  `HOST`, `PORT`, …) and 2 documented names are read in ways the grep pattern does not see
  (`LMSTUDIO_MODELS_DIR` via `os.environ.get`, `SERVICE_USERPROFILE` by the service wrapper). The clone
  documents **every** name it reads, grouped by owning crate, each with its real default from §2.1 —
  the undocumented 36 are the ones most likely to be mis-set, so they are the point of the exercise.
  Names dropped by §7.1 are not carried (`EMAIL_*` x5, `USE_WAITRESS` + `WAITRESS_{THREADS,CHANNEL_TIMEOUT}`
  x3, `MCTRADER_BASE_URL` / `MCTRADER_API_TOKEN` x2), so the clone's `.env.example` ends up around 140 keys,
  every one of them actually read, which is the opposite of the source's 113-documented / 150-read split.
- Principle 24 fixes found in the source: `lm_engine_COMPLETION_NUM_PREDICT` (`agent.py:403`) is not
  UPPERCASE; `HOST`/`PORT` are read as fallbacks behind `FLASK_HOST`/`FLASK_PORT` (`574–575`) — the clone
  uses `APP_HOST`/`APP_PORT` only; the two percent keys are bare floats
  (`ENTITY_MARKET_MOVE_PERCENT=2.0`, `ENTITY_FULL_URGENCY_MOVE_PERCENT=8.0`).
- Principle 24 in the clone: every key UPPERCASE; percentage keys end in `_PERCENT` and the value is a
  double-quoted string ending in `%` — `ENTITY_MARKET_MOVE_PERCENT="2.0%"`,
  `ENTITY_FULL_URGENCY_MOVE_PERCENT="8.0%"`. `AppConfig` parses those into `f64` fractions once, at
  startup, and no other crate sees a percent string.
- Every other numeric key stays a bare number (`ENTITY_TICK_INTERVAL_MIN_SECONDS=45`) and every
  boolean stays `true`/`false` parsed by the single `parse_bool` equivalent.
- Startup validates the whole config before any thread/task starts (principle 14): unknown key →
  warning, invalid value → hard failure with the key named. `STORAGE_ROOT` is created and checked
  before the mind loop, housekeeper or cache are constructed.
- The unprompted-entity flags keep their source defaults, off: `ENTITY_PROACTIVE_ENABLED=false`,
  `ENTITY_SLOW_LOOP_ENABLED=false`, `ENTITY_SAY_ENABLED=false`, `ENTITY_SSE_ENABLED=false`,
  `ENTITY_DRY_RUN=true`. They are one opt-in chain, not four independent switches, and
  `/api/entity/state` publishes them so the client can stand the stream up or down. A stage-7 demo that
  the entity speaks on its own is a configured state, and the config that produces it is written down.
- Secrets (`OPENAI_API_KEY`, mctrader login) come from the environment or the Windows credential
  store via the `keyring` crate, never from a file in the repo. `GET /api/keyring` reports the
  process account and whether a credential exists — never the credential.

---

## 6. `How_To_Code.md` translated into Rust gates

| Principle | How it is enforced in this workspace |
|---|---|
| 1 — complete replacement, no coexistence | One implementation per behaviour. The Python app is never called at runtime. No `#[deprecated]` shims, no compat handlers for routes the frozen client does not call (§7). Fallbacks are kept only where principle 1 allows them: external systems (LM Studio, Edge TTS, mctrader, DuckDuckGo, IMAP). |
| 2, 5 — atomic, short functions | `[workspace.lints]` denies `clippy::too_many_lines` with `clippy.toml: too-many-lines-threshold = 50`. A function that must exceed it gets an `#[allow(clippy::too_many_lines)]` plus a one-line justification in the code — and the justification is reviewed at the stage gate. |
| 3, 12 — strong types, concrete results | `serde` structs/enums for every request, response, record and tool payload. `serde_json::Value` is banned outside `shed-json` and the MCP argument layer, which is the one place genuinely dynamic data enters. Newtype wrappers for `ClientId`, `DeliveryId`, `JobId`, `CharacterId`, `ModelName`, `Seconds`, `Percent`. |
| 4, 10 — composition, no god objects | The crate map in §3.2 with the dependency direction in §3.3. `agent.py`'s 730 top-level nodes become collaborators behind small facades (`OutboxStore`, `TtsAudioCache`, `MindState`, `ToolRegistry`, `LlmClient`). |
| 6 — encapsulation | Crate-private modules; `pub` only what another crate genuinely needs. `#[cfg(test)]`-only accessors instead of `pub` for tests. |
| 7 — prefer pure functions | The 24 `utils/` modules are already pure and port as free functions. State lives in named owners (`OutboxStore`, `PendingTrades`, `SseRegistry`), never in globals. |
| 8 — business logic away from I/O | Each crate splits `logic` (pure) from `io` (reqwest/tokio/fs). HTTP tests run against a fake upstream (`shed-testutil`), never the real LM Studio. |
| 9 — descriptive single-purpose names | `build_ / validate_ / load_ / save_ / parse_ / normalize_ / select_ / compute_`. Python's `_`-prefix convention disappears; Rust visibility replaces it. Names containing `and_` are a review flag. |
| 11, 19 — one authoritative implementation | Each constant and rule exists in exactly one crate; `docs/ARCHITECTURE.md` records the owner. The duplicated vetoes and duplicated normalisation passes found in `agent.py` collapse into single implementations (see §7 improvements). |
| 13 — test behaviour | Tests assert HTTP responses, JSON payloads, file contents and function outputs. No test reaches into a private helper. The 8 JS suites test the client against the real server. |
| 14 — validate before expensive work | Config validation, storage-root creation, model availability check and voice selection all happen before a model call or a synthesis. `synthesize_speech` refuses before opening a WebSocket when the voice catalogue is empty. |
| 15 — tests for every reusable component | §9 maps all 49 Python test modules to Rust tests; a crate with no test fails its stage gate. |
| 16 — no dead code | `[workspace.lints]`: `dead_code = "deny"`, `unused = "deny"` (as a group, so `unused_*` is covered), `unused_crate_dependencies = "deny"`, `unsafe_code = "deny"`, `clippy::all = "deny"`. `clippy::pedantic` is left off as a group because `cargo clippy -D warnings` would promote it to a failure and it includes lints this project does not want (`missing_errors_doc`, `must_use_candidate`); the pedantic lints that are wanted are named individually. `cargo clippy --workspace --all-targets -- -D warnings` is a stage gate. Dropped routes/functions are listed in §7 rather than left unwired. |
| 17 — explicit side effects | I/O functions return results and are named `load_/save_/append_/publish_`. No function mutates a global; shared state is behind a named owner passed in. |
| 18 — readability | `rustfmt.toml: max_width = 100`; one `match` per decision; comments explain *why* (the source's own docstrings are worth keeping — several are load-bearing design notes). |
| 20 — verify before completion | Every stage gate runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, the JS suites, and the parity checklist diff. Temporary/one-off scripts are deleted before the stage closes. |
| 21 — delete the plan | When the last stage closes, `TODO/PLAN-Rust-Clone.md` is deleted and deferred **coding** items move to `TODO/Loose-Ends.md`, each separated by a line of `********************************`. |
| 22 — column sizing | The clone stores JSON files, not tables; the equivalent rule is applied to the JSONL/JSON records: bounded fields (`MAX_TEXT_CHARS`, `MAX_GOAL_TEXT_CHARS`, `MAX_EVENT_TEXT_CHARS`) keep their source limits and nothing unbounded is persisted. |
| 23 — guard catalogue | `docs/GUARD_CATALOGUE.md`: one entry per guard (name, owning crate+function, what it gates, what fires it, what clears it, config key, tests, ~5-line note). The clone's guards include the ~20 vetoes in `utils/entity_loop.py`, the speech-budget caps, quiet hours, reconnect throttle, SSE subscriber cap, tool auth/arming gates, trade confirmation, `is_public_web_url` private-address block, `enforce_persona_reply`, the repeat guard, and the outbox TTLs. `crates/shed-gates/src/guards.rs` fails if an entry's location no longer exists or a known guard has no entry (that gate module lands with the guards it covers, §8 Stage 8). |
| 24 — config naming | §5. |
| 25 — reasoning | Read the affected Python region and its call chain before porting it; the region map in §2.1 is the trace index. |

---

## 7. Deliberate divergences

Anything not on this list is a parity bug. Each row is also a row in `docs/PARITY.md`.

### 7.1 Surfaces the clone does not carry

| Surface | Why it is dropped | Evidence |
|---|---|---|
| `GET /_routes` | Flask introspection artifact | 0 references in `index.html`/`app-helpers.js` |
| `GET /api/debug-voices`, `GET /api/debug-models` | Duplicates of `/api/voices` and `/api/models` | 0 references |
| `POST /api/set-model` | Legacy mode-switch leftover; the launchers it served are gone | 0 references |
| `MCTRADER_BASE_URL`, `MCTRADER_API_TOKEN` | Read at 462–463 and never used anywhere else | grep: 1 occurrence each |
| `initialize_mctrader_credentials` | Body is `pass` (11519–11521) | read |
| `powershell_command` + `fetch_AI_MODEL_names` | PowerShell model discovery; `discover_lm_studio_models_from_disk` (1327–1352) already does it natively and is the path the app actually depends on | read |
| `tests/test_eodhd_screener_*` and `utils/eodhd_screener.py` | Already deleted from the source; only `.pyc` remains | dir listing |
| `last_model_index.txt` in the repo root | Moved under `STORAGE_ROOT` — runtime state does not belong in the repo | principle 19 |
| `tool_list_recent_emails`, `tool_read_email`, `email_connection`, `list_email_messages`, `fetch_email_by_uid`, `extract_email_body`, `decode_mime_header` (`agent.py:5076–5225`) | **decided**: IMAP is a separate concern; its five `EMAIL_*` keys are read by the code and documented nowhere | grep: `EMAIL_FOLDER`, `EMAIL_IMAP_HOST`, `EMAIL_IMAP_PORT`, `EMAIL_PASSWORD`, `EMAIL_USERNAME` absent from `.env.example` |
| `USE_WAITRESS` / `WAITRESS_THREADS` / `WAITRESS_CHANNEL_TIMEOUT` as a toggle | **decided**: one server (tokio + axum). Thread and channel limits become tokio/axum server config, not a mode switch | `agent.py:576–578`, `11585–11592` |
| Flask/Jinja rendering | The frozen `index.html` needs exactly 3 substitutions; a substitution step replaces Jinja | `agent.py:8419–8426` |

Two surfaces were undecided when this plan was written; both are now **decided** (§11 records the answers):
the IMAP email tools are **dropped**, and `POST /api/test-speak` is **ported** — it is the voice audition
`MARKUP_TESTING.md` describes, it was broken (bound to the wrong function, so it answered 500 to every
request) until `tests/test_test_speak_route.py` pinned it, and it has no button in the frozen client, so
the ported suite plus manual calls are what keep it honest.

### 7.2 Structural improvements allowed by D6

| Change | Reason |
|---|---|
| One `AppConfig` struct replaces 60 scattered env reads and the derived constants | principle 19; today a default is repeated wherever a value is read |
| `run_agent` (332 lines) and `call_lm_engine` (324 lines) decomposed into plan → call → parse → recover → persist | principle 2/5; today a failure path is re-implemented inside every stage |
| The retry ladder (`_should_retry_*`, fallback model, temperature, tools, json-mode, reasoning-effort, max-completion-tokens) becomes one ordered `RetryStrategy` chain | principle 11; the same decision is currently expressed in several places |
| The ~20 vetoes in `utils/entity_loop.py` become named guard functions with catalogue entries | principle 23 |
| `synthesize_speech` (88 lines) splits into voice choice → provider call → audio validation → cache admission | principle 2; today cache admission and cleanup are entangled with synthesis |
| Reply-document parsing (`ast.literal_eval` acceptance) becomes strict JSON with an explicit, catalogued repair step | principle 3/18; `literal_eval` on model output is a footgun |
| `build_tts_audio_cache` + `TtsAudioCache` + the temp-folder delete thread become one cache owner with a single delete path | principle 17; today deletion lives in a daemon thread inside a route handler |
| Conversation locks, chat-slot lock, `ACTIVE_CHAT_REQUESTS` counter and `LAST_TURN_ENDED_AT` become one `TurnGate` owner | principle 6/17; today four globals must stay in step |
| `index.html` rendering keeps the three substitutions but the template is served from `templates/` with a tiny substitution step instead of Jinja | no Jinja dependency; the client is unchanged |

### 7.3 What must **not** change

§4's contract, the outbox/SSE delivery semantics (queue **and** push), the single-turn gate, the
`deferred` 202 + job-polling flow, the busy reply text, the `sentences[]` streaming shape, the
`text` vs `text_markup` split, and the frozen client files.

---

## 8. Stages

Each stage ends with: `cargo fmt --check` → `cargo clippy -D warnings` → `cargo test --workspace` →
JS suites green → `docs/PARITY.md` updated → guard catalogue updated → no temp files left.

| Stage | Deliverable | Exit gate (beyond the standard one) |
|---|---|---|
| **0** | Workspace, lints, `rustfmt.toml`, `clippy.toml`, `package.json`, `docs/` skeleton, the client and data copied byte-for-byte into the mirrored layout (`templates/`, `static/`, `styles/`, `characters/`, `prompts/`, `outputs/tts/`, `voices.json`) with a hash manifest, `shed-gates` (frozen-asset + layering + env-read gates), `npm test` wired to `tests/*.js`, `.gitignore` | the `shed-gates` asset tests pass (144 entries, 0 drift); the 19 empty crates compile under `-D warnings`; the 8 JS suites pass unchanged |
| **1** | `shed-config`, `shed-json`, `shed-text`, `shed-store`, `shed-logging`; binary boots and serves `GET /health` + `X-Request-ID` + error envelope | `/health` byte-comparable; `test_env_utils`, `test_json_util_utils`, `test_text_utils`, `test_entity_store_utils` ported |
| **2** | `shed-models` + `shed-llm`: discovery (disk + API), cache persistence, context windows, transport with retry, streaming reader, prompt-block store | `test_models_utils`, `test_lm_router_utils`, `test_lm_streaming` ported; a fake-LM test proves the retry ladder |
| **3** | `shed-prompts`, `shed-characters` + catalogue routes (`/`, `/app.css`, `/app-helpers.js`, webp, `/api/countries`, `/api/characters`, `/api/voices`, `/api/models`, last-model-index, `/api/refresh-models`), warmer task | The frozen client loads and the country→character→voice flow works end-to-end against the real `characters/` data |
| **4** | `shed-conversation` + `shed-chat`: records, identity, compaction, summarisation, housekeeping, recall/persona guardrails, reply shaping, `JsonReplyStreamScanner`, `SentenceSplitter`, `ReplyStreamPublisher`, `run_agent`, `POST /api/chat` + `/api/chat/jobs/<id>` + the turn gate | `test_api_chat`, `test_conversation_*`, `test_reply_*`, `test_sentence_splitter`, `test_reply_stream_*`, `test_glm4*` ported; a chat turn streams sentences to the client |
| **5** | `shed-tools` + `shed-mcp`: local tools, DuckDuckGo search/fetch, spreadsheet writer, trade preview + confirmation, MCP stdio client, mctrader tool inference and auth gating, `/api/mcp/auth`, `/api/mcp/tools`, `/api/mcp/tools/refresh` | `test_mcp_*`, `test_pending_trades`, `test_trade_confirmation_flow` ported; a tool call round-trips through a fake MCP server |
| **6** | `shed-tts` (voice catalogue, Edge WebSocket + `Sec-MS-GEC` v2 token, Windows SAPI, audio cache, MP3 pause trimming, failure messages) wired into `shed-http` as `/api/speak`, `/api/reply-laugh`, `/api/reply-fart`, `/api/test-speak` | **Opens with a time-boxed spike** on the v2 token; if it fails, ship Windows-SAPI-only and record Edge as a Loose-End. `test_tts_*`, `test_speech_pauses`, `test_voice_tones`, `test_tts_failure_message`, `test_test_speak_route` ported |
| **7** | `shed-entity` + `shed-mind`: client records, outbox, SSE registry, slow loop, events, goals, drives, user model, wired as the `/api/entity/*` routes incl. the stream | `test_entity_*`, `test_mind_*` (incl. the three `_wiring` suites), `test_outbox`, `test_event_stream` ported; the entity speaks unprompted to the frozen client through SSE **with the default-off opt-in chain switched on** (`ENTITY_PROACTIVE_ENABLED`, `ENTITY_SLOW_LOOP_ENABLED`, `ENTITY_DRY_RUN=false`, `ENTITY_SSE_ENABLED`) — and a second run with the defaults proves the off state is unchanged |
| **8** | Ops: `/api/restart`, `/api/unload-models`, `/api/keyring`, cleanup scheduler, retention, `logs`, the Windows service installer pair, cloudflared service | The app runs as a Windows service with no Python installed; reboot restores state |
| **9** | Full parity sweep: every `docs/PARITY.md` row ticked or justified, guard catalogue complete, `docs/` current, plan deleted per principle 21 | 49 ported test modules + 8 JS suites green against the Rust server; a side-by-side run of the Python app and the Rust app produces the same JSON for every route |

---

## 9. Test port map (49 Python test modules + `conftest.py` + 8 JS suites)

| Destination crate | Ported from |
|---|---|
| `shed-config` | `test_env_utils` |
| `shed-json` | `test_json_util_utils` |
| `shed-text` | `test_text_utils` |
| `shed-store` | `test_entity_store_utils` |
| `shed-models` | `test_models_utils`, `test_lm_router_utils`, `test_agent_models` |
| `shed-llm` | `test_lm_streaming` |
| `shed-prompts` | `test_prompt_utils`, `test_prompt_cache` |
| `shed-characters` | `test_characters`, `test_character_profiles` |
| `shed-conversation` | `test_conversation_history_utils`, `test_conversation_housekeeping`, `test_conversation_identity` |
| `shed-chat` | `test_reply_utils`, `test_reply_recovery`, `test_reply_stream_scanner`, `test_reply_stream_publisher`, `test_sentence_splitter`, `test_glm4`, `test_glm4_followup` |
| `shed-tools` | `test_pending_trades`, `test_trade_confirmation_flow` |
| `shed-mcp` | `test_mcp_utils`, `test_mcp_args_utils`, `test_mcp_argument_guard` |
| `shed-tts` | `test_tts_utils`, `test_tts_cache`, `test_tts_failure_message`, `test_speech_pauses`, `test_voice_tones` |
| `shed-entity` | `test_entity_client`, `test_outbox`, `test_event_stream` |
| `shed-mind` | `test_entity_loop`, `test_mind_loop`, `test_mind_events`, `test_mind_goals`, `test_mind_state`, `test_mind_user`, `test_mind_events_wiring`, `test_mind_state_wiring`, `test_mind_user_wiring` |
| `shed-http` | `test_api_chat`, `test_http_server`, `test_app_stylesheet`, `test_frontend_cache_headers`, `test_test_speak_route` |
| `shed-testutil` | `conftest.py` fixtures → temp storage roots, fake LM Studio, fake MCP server, fake Edge endpoint |
| `tests/*.js` | `test_entity_client.js`, `test_mobile_characters.js`, `test_mobile_client.js`, `test_mobile_model_selection.js`, `test_mobile_speech_chunking.js`, `test_mobile_speech_markers.js`, `test_mobile_speech_probe.js`, `test_mobile_speech_queue.js` — **unchanged**, run by `npm test` against `static/app-helpers.js` |
| `shed-gates` | no Python source: the workspace-wide gates (frozen assets, layering, env-read rule). Their tests live in each gate module, not in a `tests/` directory, because `unused_crate_dependencies` is checked per target |

Three suites (`test_mind_events_wiring`, `test_mind_state_wiring`, `test_mind_user_wiring`) test the
`agent.py` wiring rather than the util; they port as `shed-mind` integration tests driving the slow
tick, which is exactly the wiring the Rust version must reproduce.

`tests/test_glm4.py` and `test_glm4_followup.py` are model-specific regressions. They need recorded
upstream responses: Stage 4 captures golden fixtures from the Python app **before** any Python code is
retired, so the Rust port is graded against real model output rather than a hand-written guess.

---

## 10. Risks and mitigations

| # | Risk | Mitigation |
|---|---|---|
| 1 | **Edge TTS v2 `Sec-MS-GEC` token in Rust.** The source pins `edge-tts>=7.3.0` precisely because v1 tokens are refused with a 403 WebSocket handshake, and the app cannot detect or repair that | Stage 6 opens with a time-boxed spike. If it fails: ship Windows-SAPI-only, record Edge as a Loose-End, and keep the failure message contract (`describe_tts_failure`) so the user is told exactly what is missing |
| 2 | **Server semantics that axum gets differently.** The source runs Werkzeug threaded by default (`USE_WAITRESS=0`) and waitress only when opted in; either way a WSGI app may not set `Connection`, and waitress asserts on it (11458–11463) | `docs/CONTRACT.md` records the header rules; a contract test asserts the exact header set on every route, including the absence of `Connection`. The clone's single server is sized like the waitress branch (bounded workers + read timeout), which is what a long-lived stream needs |
| 3 | **Concurrency model change.** waitress threads + a global lock vs tokio tasks | `TurnGate` is one owner with a test that reproduces the busy path (HTTP 200 busy payload) under two simultaneous requests |
| 4 | **Heuristic drift.** Reply-usability, grounding, salience, recall scoring and tone-tag rules are full of quirks that the client has adapted to | Golden fixtures captured from the Python app before it is retired; the 8 JS suites run against the Rust server from Stage 3 onward, not at the end |
| 5 | **Frozen 194 KB `index.html` + 120 KB `app-helpers.js`.** Any JSON drift is a client break | Contract tests are generated from the JS helpers' expectations (`buildOutboxRequestUrl`, `buildEntityStreamUrl`, `buildEntityStateUrl`, `readOutboxMessages`, `nextOutboxPollDelayMs`) so a server-side change fails fast |
| 6 | **MCP maturity.** The source needs `mcp>=1.27.0` over stdio against the mctrader server | `shed-mcp` isolates the protocol behind one client type; a fake MCP server in `shed-testutil` is the primary gate, and the real server is an integration test only |
| 7 | **Windows service + keyring under a service account.** `MCTRADER_LOGIN_KEYRING_*` and `/api/keyring` depend on the process account | Stage 8 runs the service-installer pair and asserts `current_process_account` reports the service identity |
| 8 | **Scale.** 11,607 lines with an 890-line character catalogue and a 332-line `run_agent` | Stage order is fixed by dependency (§3.3); no stage carries more than one monolith region |
| 9 | **Model discovery drift** from LM Studio's local store layout (`lmstudio_models_root`, `ingest_discovery_item`) | Discovery is tested against a captured directory fixture, and the `models_cache.json` fallback path is ported so a discovery failure degrades the way the Python app degrades |

## 11. Stage 0 decisions — all resolved

| # | Question | Decision | What it commits |
|---|---|---|---|
| 1 | IMAP email tools | **Drop** | `agent.py:5076–5225` is not ported; the five `EMAIL_*` keys never enter this repo's `.env.example`; `TOOLS` and `TOOL_DESCRIPTIONS` lose the two email entries. Recorded in `docs/PARITY.md` as deliberately absent, restorable from `TODO/Loose-Ends.md` |
| 2 | `POST /api/test-speak` | **Port** | Route 35 lands in Stage 6 with `parse_tts_test_tuning` and `choose_tts_voice_for_request`; `tests/test_test_speak_route.py` and `tests/test_tts_failure_message.py` port unchanged; fix #16 (report the real audio mimetype) applies |
| 3 | `TTS_PROVIDER` default | **`edge`, Windows SAPI as the fallback provider** | `AppConfig.tts_provider` defaults to `edge` (matching `.env.example`, not `agent.py:557`); the provider choice is one enum with two implementations, selected per voice request, and a failed Edge call surfaces through `describe_tts_failure` rather than silently switching |
| 4 | `USE_WAITRESS` toggle | **Drop** | One server: tokio + axum, bounded workers, read/channel timeout sized to the waitress branch (`WAITRESS_THREADS=32`, `WAITRESS_CHANNEL_TIMEOUT=600`). No `USE_WAITRESS`, no Werkzeug mode |
| 5 | Web search | **Keep DuckDuckGo scraping** | `fetch_duckduckgo_search_html` (`agent.py:5307`, `html.duckduckgo.com/html/`) and `extract_duckduckgo_result_url` (5248) port as-is in `shed-tools`, with the private-address block and the `source: "duckduckgo_html"` marker the model's prompt depends on. No search API, no new key |
| 6 | Golden fixtures | **Run the Python app and capture** | Before any Python file is retired: capture route payloads, model reply documents, job `sentences[]` progress and real Edge/SAPI audio into `tests/fixtures/`. Stage 4 captures chat + model fixtures, Stage 6 captures TTS audio, Stage 7 captures mind-loop tick outputs. The Python app stays runnable until Stage 9's side-by-side sweep passes |

Two facts found while resolving these, both now carried into the stages:

- **The server default is Werkzeug, not waitress** (`USE_WAITRESS` defaults false at `agent.py:580`;
  `.env.example` ships `USE_WAITRESS=0`). §2.4 and risk 2 are written around that.
- **Everything unprompted is off by default**: `ENTITY_PROACTIVE_ENABLED`, `ENTITY_SLOW_LOOP_ENABLED`,
  `ENTITY_SAY_ENABLED`, `ENTITY_SSE_ENABLED` all default false and `ENTITY_DRY_RUN` defaults true. Stage 7
  reproduces the opt-in chain and runs its exit gate with it switched on; a Stage 7 demo that "the entity
  speaks on its own" is a configured state, not the default one.
