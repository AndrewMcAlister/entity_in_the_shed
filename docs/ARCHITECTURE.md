# Architecture

The Rust clone of Entity In The Shed: one cargo workspace, library crates per
domain, one binary (`shed-server`). The Python original (`agent.py`, 11,607
lines) is decomposed here, not transliterated.

## Workspace layout

```
Entity_In_The_Shed/
├── Cargo.toml                [workspace] + [workspace.dependencies] + [workspace.lints]
├── rustfmt.toml              max_width = 100, stable-channel options only
├── clippy.toml               too-many-lines-threshold = 50
├── package.json              npm test -> node --test "tests/*.js"
├── .env.example              every name the code reads, grown stage by stage
├── docs/                     this file, CONTRACT.md, PARITY.md, GUARD_CATALOGUE.md
├── templates/index.html      frozen (D2)   ├── characters/  prompts/  voices.json
├── static/app-helpers.js     frozen (D2)   ├── outputs/tts/{laugh,fart}/
├── styles/                   frozen (D2)   ├── crates/      (created on demand, D4)
├── bins/shed-server/         the one binary
└── tests/
    ├── test_*.js             the 8 browser suites, unchanged (D2)
    └── assets.manifest.json  path + bytes + sha256 of every frozen file
```

The layout mirrors the Python app on purpose: the JS suites
`require("../static/app-helpers.js")`, so a mirrored `tests/`-next-to-`static/`
layout needs zero client edits.

## Crates

Crates are created on demand: a crate appears only when a stage gives it a
production caller (decision D4). The layer map below is complete from day one
and `crates/shed-gates/src/layers.rs` enforces it over whatever exists.

| Crate | Source of its logic (Python) | Responsibility |
|---|---|---|
| `shed-config` | `utils/env.py` + `agent.py:376–955` | Typed `AppConfig`: every env name parsed once, validated, immutable. The only crate that reads the environment. |
| `shed-json` | `utils/json_util.py` | JSON safety, strict response parsing, token-byte helpers |
| `shed-text` | `utils/text.py` | token estimation, tone-tag/markup normalisation, vocalisation respelling, bracket segmentation |
| `shed-store` | `utils/entity_store.py` | atomic JSON/JSONL writes, quarantine + recovery, client-id mint/normalise |
| `shed-logging` | `configure_logging` (`agent.py:11524–11550`) | one logger, one format, one destination; owns the `LOG_LEVEL` mapping; never reads `RUST_LOG` |
| `shed-models` | `utils/{models,lm_router}.py`, `agent.py:1239–1724`, `6207–6497` | discovery (disk + API) + cache, context windows, display names, router unload/residency |
| `shed-llm` | `agent.py:1038–1236`, `6500–7131` | LM Studio/OpenAI-compatible transport: retry ladder, streaming reader, sampler payload, prompt-block store, response parsing |
| `shed-prompts` | `utils/{prompt,prompt_cache}.py`, `agent.py:598–702`, `2106–2193`, `7134–7247` | templates, tone/delivery maps, combined system prompt, prompt-cache plan |
| `shed-characters` | `utils/characters.py`, `agent.py:2196–3085` | country/character catalogue, profiles, outlines, sex/pronouns, roster disambiguation |
| `shed-conversation` | `utils/conversation_history.py`, `agent.py:3785–5000` | records, identity/filenames, compaction, summarisation, housekeeping, recall + persona guardrails |
| `shed-chat` | `utils/reply.py`, `agent.py:5504–6202`, `7760–8417` | reply shaping, JSON reply scanner, sentence splitter, stream publisher, `run_agent`, completeness/usability recovery |
| `shed-tools` | `agent.py:5003–5073`, `5228–5501`, `utils/pending_trades.py` | local tool registry + implementations, DuckDuckGo search/fetch with private-address block, spreadsheet writer, configured start/stop/status commands, trade preview + confirmation state machine |
| `shed-mcp` | `utils/{mcp,mcp_args}.py`, `agent.py:1727–2103`, `7304–7757` | MCP session/transport, tool normalisation, auth gating, redaction, free-text argument inference, mctrader tool-call resolution |
| `shed-tts` | `utils/{tts,tts_cache,speech_pauses}.py`, `agent.py:3092–3276`, `3476–3782` | voice catalogue + selection, Edge WebSocket synthesis, Windows SAPI synthesis, audio cache, MP3 pause trimming, failure description |
| `shed-entity` | `utils/{entity_client,mind_outbox}.py`, `agent.py:9524–9644`, `10496–10602`, `11053–11107` | client records, outbox store, SSE subscriber registry, frame formatting, utterance history |
| `shed-mind` | `utils/{entity_loop,mind_events,mind_goals,mind_state,mind_user}.py`, `agent.py:9647–10493`, `10796–11050` | slow loop, ~20 vetoes, grounding, events, goals, drives, user model, thought journal |
| `shed-http` | `agent.py:8418–9510`, `11053–11607` | axum router, middleware, error envelope, static assets, deferred chat jobs |
| `shed-testutil` | — | dev-only: temp storage roots, fake LM/MCP/Edge endpoints, fixture builders |
| `shed-gates` | — | dev-only: workspace-wide invariant gates (frozen assets, crate layering, env-read rule; settings-template and guard-catalogue gates arrive later) |
| `shed-server` (bin) | `agent.py:11553–11592` | startup order, background tasks, graceful shutdown, `--smoke` boot gate |

## Dependency direction (never violated; gate-checked)

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
shed-mind → config, entity, models, llm, prompts, conversation, tools
shed-http → the 16 above (composition root, apart from the binary)
shed-server → everything, including shed-http
shed-testutil → the 16 above (dev-dependency only)
shed-gates → shed-config, shed-testutil (dev-dependency only; no shipped crate may depend on either)
```

No cycles; no crate depends on a crate that composes it. `shed-testutil` may
only ever appear under `[dev-dependencies]`; `shed-gates` has no consumers at
all — its tests run inside its own package.

## Ownership rules

- One implementation per behaviour; each constant/rule exists in exactly one
  crate. This table names the owner of each domain; a rule moving crates is a
  change to this file.
- Shared state lives in named owners passed in (`OutboxStore`, `TtsAudioCache`,
  `MindState`, `ToolRegistry`, `LlmClient`, `TurnGate`), never in globals.
- Each crate splits pure logic from I/O; HTTP tests run against fakes in
  `shed-testutil`, never the real LM Studio.

## Runtime shape

One tokio + axum server sized like the Python app's waitress branch (32
workers, 600 s channel timeout) — the original's `USE_WAITRESS` toggle is not
cloned. Background work (cleanup scheduler, conversation housekeeper,
character warmer, mind loop, deferred chat jobs, profile workers) runs as
named tokio tasks owned by `shed-server`. Startup order follows the original:
logging → model discovery → schedulers → banner → bind.

Deployment (decision D9): the Windows service is named `entity_in_the_shed`
and is designed to run **alongside** the Python service
(`entity-in-the-shed`): its default `STORAGE_ROOT` is
`C:\Temp\entity_in_the_shed` (Python: `C:\Temp\entity-in-the-shed`) so two
live services never write the same files, and the installer must detect a
port clash (both default to 5005) instead of failing at boot.
