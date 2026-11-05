# Entity In The Shed (Rust)

A native Rust rewrite of the Python/Flask app at `D:\Entity-In-The-Shed`:
one cargo workspace, one binary (`shed-server`), no Python at runtime.

Status: **Stage 0 complete** (workspace skeleton, frozen client, gates).
The staged plan lives in `TODO/PLAN-Rust-Clone.md` until coding finishes.

## Read first

- `docs/ARCHITECTURE.md` — crate map and dependency direction
- `docs/CONTRACT.md` — the frozen HTTP/SSE contract the server must satisfy
- `docs/PARITY.md` — every deliberate divergence from the Python app
- `docs/GUARD_CATALOGUE.md` — index of all guards (principle 23)
- `How_To_Code.md` — the coding principles every change obeys

## Frozen files

`templates/`, `static/`, `styles/`, `characters/`, `prompts/`, `voices.json`,
`outputs/tts/` and `tests/*.js` are copied byte-for-byte from the Python app
and never edited. `crates/shed-gates` compares them against
`tests/assets.manifest.json` on every `cargo test`.

## Running the tests

```
cargo test --workspace     # Rust tests, including the workspace gates
npm test                   # the 8 browser suites (250 tests) against static/app-helpers.js
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Requires Rust (stable) and Node >= 22. No npm dependencies.

## Deploying alongside the Python service

The Windows service is named `entity_in_the_shed` (Python:
`entity-in-the-shed`) and its default `STORAGE_ROOT` is
`C:\Temp\entity_in_the_shed`, so both apps can coexist; the Stage 8
installer detects port clashes (both default to 5005). See decision D9.
