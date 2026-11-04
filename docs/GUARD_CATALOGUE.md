# Guard catalogue

`How_To_Code.md` principle 23. A guard is gating logic that is more than a one-line calculation: a rule
that can block, veto, cap, clamp, delay or force an action, and that reads better as a named function
than as several lines inline. Every guard has exactly one entry here: its name, where it lives, what it
gates, what fires it, what clears it, its configuration key, its tests, and a note of about five lines.

The catalogue is an index, not a home: guard code stays with the crate that owns the state it reads.

## 1. Gates that exist today

These are workspace-wide gates rather than app guards — they gate the repository itself, so they live in
`crates/shed-gates`, which no shipped crate depends on. Each gate module carries its own tests, because
`unused_crate_dependencies` is checked per target and an integration test would link the package's own
dependencies.

### `frozen_assets`

- **Lives in**: `crates/shed-gates/src/assets.rs`
- **Gates**: the browser client, the data it reads and the eight browser suites (`templates/`,
  `static/`, `styles/`, `characters/`, `prompts/`, `outputs/`, `voices.json`, `tests/test_*.js`).
- **Fires when**: a frozen file's bytes or size differ from `tests/assets.manifest.json`, a manifest
  entry is missing from disk, or a new file appears inside a frozen tree.
- **Clears when**: the file matches the manifest again. The manifest is regenerated only by an explicit
  decision to unfreeze something, which is a contract change and belongs in `docs/PARITY.md`.
- **Config key**: none.
- **Tests**: `assets::tests::every_manifest_entry_is_on_disk_and_unchanged`,
  `assets::tests::nothing_extra_appears_in_a_frozen_tree`,
  `assets::tests::the_eight_browser_suites_are_present_and_frozen`.
- **Note**: the client is copied byte-for-byte from the Python app and never edited, because the Rust
  server has to satisfy the client that already exists (decision D2). The manifest holds 144 entries —
  136 client and data files plus the 8 suites — with path, byte count and SHA-256 each. The layout
  mirrors the Python app so the suites' `require('../static/app-helpers.js')` resolves untouched.

### `crate_layering`

- **Lives in**: `crates/shed-gates/src/layers.rs`
- **Gates**: the dependency direction of every package in the workspace (`docs/ARCHITECTURE.md` §3).
- **Fires when**: a `Cargo.toml` lists a dependency its layer may not reach, a package is missing from
  the layer table, or a layer-table entry has no package.
- **Clears when**: the edge is removed or the table is updated deliberately, in the same commit as the
  reason.
- **Config key**: none.
- **Tests**: `layers::tests::no_crate_depends_outside_its_layer`,
  `layers::tests::every_workspace_package_is_declared_in_the_layer_table`.
- **Note**: Cargo does not check dependency direction and clippy cannot express it, so the rules live as
  data (`LAYERS`, 20 entries) and a test reads each package manifest against them. A violation names the
  crate and the edge instead of "layering broke". `shed-testutil` and `shed-gates` are test-only and no
  shipped crate may depend on them.

### `single_env_reader`

- **Lives in**: `crates/shed-gates/src/layers.rs`
- **Gates**: who may read the environment (principles 19 and 24).
- **Fires when**: any crate other than `shed-config` contains `env::var(`, `env::vars(`, `env::var_os(`
  or `dotenvy::`.
- **Clears when**: the value is threaded through `AppConfig` instead.
- **Config key**: none.
- **Tests**: `layers::tests::only_shed_config_reads_the_environment`,
  `layers::tests::the_env_scan_catches_a_read_and_ignores_the_marker_table`.
- **Note**: `env!(...)` is a compile-time macro and is not a read, so it is not matched. The scan blanks
  the inside of string and raw-string literals first, because the gate's own marker table and its test
  fixtures contain the markers as text; a real read still reads as `env::var("")` and is caught.

## 2. App guards, not yet ported

Each of these exists in the Python source and gets a full entry above when its stage lands. The stage
gate fails if a stage ships a guard without its entry.

| Guard | Source | Lands in | Stage |
|---|---|---|---|
| The ~20 slow-loop vetoes | `utils/entity_loop.py` | `shed-mind` | 7 |
| Speech budget caps (per hour / per day) | `agent.py` `ENTITY_MAX_UTTERANCES_PER_*` | `shed-mind` | 7 |
| Quiet hours | `agent.py` slow loop | `shed-mind` | 7 |
| Reconnect throttle (`429 entity_reconnect_too_fast`) | `agent.py:11474–11499` | `shed-entity` | 7 |
| SSE subscriber cap (`503 entity_stream_full`) | `agent.py:11474–11499` | `shed-entity` | 7 |
| Outbox TTLs and drain-until-ack | `utils/mind_outbox.py` | `shed-entity` | 7 |
| Repeat guard (do not re-speak the same line) | `agent.py` slow loop | `shed-mind` | 7 |
| Persona guardrail (`enforce_persona_reply`) | `agent.py` | `shed-conversation` | 4 |
| Conversation single slot (`ACTIVE_CHAT_REQUESTS` + lock) | `agent.py:9443–9510` | `shed-chat` | 4 |
| Tool auth / arming gates | `utils/mcp.py`, `agent.py:1727–2103` | `shed-mcp` | 5 |
| Trade preview + confirmation state machine | `utils/pending_trades.py` | `shed-tools` | 5 |
| `is_public_web_url` private-address block | `agent.py` web fetch | `shed-tools` | 5 |
| Model unload / residency rules | `utils/lm_router.py` | `shed-models` | 2 |

`crates/shed-gates/src/guards.rs` will hold the test that keeps this catalogue honest — an entry whose
location no longer exists, or a guard that has no entry, fails the suite. That gate module lands with the
guards it covers (§8 Stage 8), so the catalogue is honest about what is not yet ported rather than
listing code that does not exist.
