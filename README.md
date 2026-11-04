# The Entity In The Shed (Rust)

The Rust port of the Python/Flask app at `D:\Entity-In-The-Shed`. A cargo workspace: one crate per
domain, one binary, no Python at runtime.

The browser client is **not** rewritten. `templates/`, `static/`, `styles/`, `characters/`, `prompts/`,
`outputs/tts/` and `voices.json` are copied byte-for-byte from the Python app and frozen by a test, so
the Rust server has to satisfy the HTTP/SSE contract the client already speaks. That contract is
[`docs/CONTRACT.md`](docs/CONTRACT.md).

## Read these first

| Document | Contents |
|---|---|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | crate map, the fixed dependency direction, where runtime state lives |
| [`docs/CONTRACT.md`](docs/CONTRACT.md) | the frozen HTTP/SSE contract, verified against the Python source |
| [`docs/PARITY.md`](docs/PARITY.md) | one row per surface of the Python app: ported, dropped with a reason, or not yet reached |
| [`docs/GUARD_CATALOGUE.md`](docs/GUARD_CATALOGUE.md) | principle 23 index of every guard |
| [`TODO/PLAN-Rust-Clone.md`](TODO/PLAN-Rust-Clone.md) | the porting plan. Deleted when the last stage closes (principle 21) |
| [`How_To_Code.md`](How_To_Code.md) | the 25 principles this project is written under |

## Layout

```
Cargo.toml            workspace, shared dependencies, shared lints
rustfmt.toml          max_width = 100, stable-channel options only
clippy.toml           too-many-lines-threshold = 50 (principle 5)
package.json          npm test → node --test "tests/*.js"
crates/               19 library crates (shed-config … shed-gates)
bins/shed-server/     the one binary
templates/ static/ styles/ characters/ prompts/ outputs/ voices.json
                      the frozen client and data
tests/                the 8 browser suites, unchanged, plus assets.manifest.json
docs/                 architecture, contract, parity, guard catalogue
```

## Toolchain

`cargo` / `rustc` 1.99, `node` 22 or newer (the suites use `node --test`), npm. No Python is needed to
build or run the Rust app; the Python app exists only until the port is finished, to capture golden
fixtures and to compare behaviour.

## The stage gate

Every stage closes with all of these, in this order:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
cargo run -p shed-server -- --smoke     # from Stage 1
```

then `docs/PARITY.md` and `docs/GUARD_CATALOGUE.md` are updated and any one-off script used during the
stage is deleted (principle 20). `cargo clippy` runs with `-D warnings`, so a warning is a build failure.

## Stages

| Stage | Deliverable |
|---|---|
| 0 | workspace, lints, frozen assets + hash manifest, `shed-gates`, docs skeleton — **done** |
| 1 | `shed-config`, `shed-json`, `shed-text`, `shed-store`, `shed-logging`; `/health`, `X-Request-ID`, error envelope, `--smoke` |
| 2 | `shed-models`, `shed-llm`: discovery, cache, transport with retry, streaming |
| 3 | `shed-prompts`, `shed-characters` + the catalogue and static routes |
| 4 | `shed-conversation`, `shed-chat` + `POST /api/chat` and its job polling |
| 5 | `shed-tools`, `shed-mcp` + MCP routes |
| 6 | `shed-tts`: Edge WebSocket synthesis with the v2 `Sec-MS-GEC` token, Windows SAPI, audio cache |
| 7 | `shed-entity`, `shed-mind` + the `/api/entity/*` routes and the SSE stream |
| 8 | ops: restart, unload, keyring, cleanup scheduler, retention, the Windows service pair |
| 9 | full parity sweep, guard catalogue complete, plan deleted |

## Running it

The server does not exist yet — `bins/shed-server` is a stub until Stage 1 boots it. What runs today:

```sh
cargo test --workspace     # the shed-gates invariants: frozen assets, layering, env-read rule
npm test                   # the 8 browser suites against static/app-helpers.js
```

Runtime state never lives in this repository. `STORAGE_ROOT` defaults to
`%LOCALAPPDATA%\entity-in-the-shed` and holds conversations, outbox, TTS cache, logs and quarantine
(principle 19). `.env` is never committed; `.env.example` is, and every key in it is UPPERCASE with
percent values written as `"5.0%"` (principle 24).
