# Architecture

The Rust clone of *The Entity In The Shed*. One cargo workspace, one crate per domain, one binary.
This document is the authority on where code lives and which direction a dependency may point;
`crates/shed-gates/src/layers.rs` holds the same table as data and a test enforces it.

## 1. Shape

```
Entity_In_The_Shed/
├── Cargo.toml                [workspace] + [workspace.dependencies] + [workspace.lints]
├── rustfmt.toml              max_width = 100, stable-channel options only
├── clippy.toml               too-many-lines-threshold = 50 (principle 5)
├── package.json              npm test → node --test "tests/*.js"
├── .env.example              every key the app reads, principle 24 (Stage 1)
├── templates/ static/ styles/ characters/ prompts/ outputs/tts/ voices.json
│                             the frozen client and data, byte-for-byte from the Python app
├── crates/                   19 library crates
├── bins/shed-server/         the one binary
├── docs/                     this file, CONTRACT.md, PARITY.md, GUARD_CATALOGUE.md
└── tests/                    the 8 browser suites (unchanged) + assets.manifest.json
```

The repository layout mirrors the Python app rather than using an `assets/` or `data/` folder. The
eight browser suites do `require('../static/app-helpers.js')`, so only the mirrored layout lets them
run byte-unchanged, and the client is copied byte-unchanged on purpose (decision D2): the Rust server
must satisfy the client that already exists.

## 2. Crates (19 libraries + 1 binary)

| Crate | Owns |
|---|---|
| `shed-config` | Typed `AppConfig`: every environment name parsed once, validated, immutable. The only crate that reads the environment. |
| `shed-json` | JSON safety, strict response parsing, token-byte helpers |
| `shed-text` | token estimation, tone-tag/markup normalisation, vocalisation respelling, bracket segmentation |
| `shed-store` | atomic JSON/JSONL writes, quarantine + recovery, client-id mint/normalise |
| `shed-logging` | one logger, one format, one destination |
| `shed-models` | model discovery + cache, discovery-from-disk, context windows, display names, router unload/residency |
| `shed-llm` | LM Studio/OpenAI-compatible transport: retry, streaming, sampler payload, prompt-block store, response parsing |
| `shed-prompts` | prompt templates, tone/delivery maps, combined system prompt assembly, prompt-cache plan |
| `shed-characters` | country/character catalogue, profiles, outlines, sex/pronoun handling, roster disambiguation |
| `shed-conversation` | conversation records, identity/filenames, compaction, summarisation, housekeeping, recall + persona guardrails |
| `shed-chat` | reply shaping, JSON reply scanner, sentence splitter, stream publisher, the agent turn, completeness/usability recovery |
| `shed-tools` | local tool registry + implementations, spreadsheet writer, `DuckDuckGo` search/fetch, configured start/stop/status commands, trade preview + confirmation |
| `shed-mcp` | MCP session/transport, tool normalisation, auth gating, redaction, argument inference, mctrader tool-call resolution |
| `shed-tts` | voice catalogue + selection, Edge WebSocket synthesis, Windows SAPI synthesis, audio cache, MP3 pause trimming, failure description |
| `shed-entity` | client records, outbox store, SSE subscriber registry, frame formatting, utterance history |
| `shed-mind` | slow loop, vetoes, grounding, events, goals, drives, user model, thought journal |
| `shed-http` | axum router, the ported routes, middleware, error envelope, static assets, deferred chat jobs. The composition root. |
| `shed-testutil` | dev-dependency only: temp storage roots, fake LM/MCP/Edge servers, fixture builders |
| `shed-gates` | dev-dependency only: the workspace-wide gates (frozen assets, layering, the env-read rule) |
| `shed-server` (bin) | startup order, background tasks, graceful shutdown, `--smoke` |

`shed-gates` owns no domain logic and no shipped crate depends on it. A gate that spans the whole
workspace belongs to no single domain crate, so it lives there instead.

## 3. Dependency direction

An edge is allowed only if it appears below. Cargo does not check direction and clippy cannot express
it, so `shed-gates`'s layering test reads every package's `Cargo.toml` and fails on any edge not
listed here.

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
shed-http          → the 16 crates above (the composition root, apart from the binary)
shed-server (bin)  → everything, including shed-http
shed-testutil      → the 16 crates above (dev-dependency only)
shed-gates         → shed-config, shed-testutil (dev-dependency only)
```

Three rules follow from that table and are tested with it:

- **No cycles.** A crate may not depend on a crate that composes it.
- **Test-only crates stay test-only.** `shed-testutil` and `shed-gates` are dev-dependencies; no
  shipped crate depends on them at runtime.
- **Only `shed-config` reads the environment** (principles 19 and 24). Every other crate is handed an
  `AppConfig`. The scan blanks the inside of string literals first, so the gate's own marker table
  cannot match itself.

## 4. Runtime state

Nothing the app writes at runtime lives in the repository. `STORAGE_ROOT` defaults to
`%LOCALAPPDATA%\entity-in-the-shed` (the Python app used a folder inside its own tree), and
conversations, outbox, TTS cache, logs and quarantine all live under it. `.gitignore` keeps it out.

## 5. Stage gate

Every stage closes with, in order:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
cargo run -p shed-server -- --smoke      (from Stage 1)
```

plus a `docs/PARITY.md` update, a `docs/GUARD_CATALOGUE.md` update, and no one-off scripts left
behind (principle 20).
