# Guard Catalogue (principle 23)

One entry per guard: what it protects, where it lives, what fires and clears
it, its config key, and the tests that pin it. `crates/shed-gates` will fail
on a stale entry location or a known guard with no entry (that gate check
arrives with the first ported guard).

Entry format:

```
## <Guard name>
- Owner: <crate>::<module>::<function>
- Gates: <what action is refused/degraded when it fires>
- Fires: <condition>   Clears: <condition>
- Config: <ENV_KEY(s) + defaults>
- Tests: <test names>
- Note: <~5 lines of why this exists>
```

Statuses: **pending (Stage N)** = exists in the Python source, not yet
ported; **ported** = entry filled with live Rust locations and tests.

## Known guards (from the Python source; entries fill in as stages port them)

| Guard | Owner lands in | Status |
|---|---|---|
| Single conversation turn slot (busy reply) | `shed-chat::TurnGate` | pending (Stage 4) |
| Chat job TTL + sentence cap | `shed-chat` | pending (Stage 4) |
| Persona enforcement on replies (`enforce_persona_reply`, `agent.py:4886`) | `shed-conversation` | pending (Stage 4) |
| Tool auth gating + arming (`/api/mcp/auth`) | `shed-mcp` | pending (Stage 5) |
| Trade confirmation + code expiry + price-move refusal | `shed-tools` | pending (Stage 5) |
| Private-address block for web fetch (`is_public_web_url`) | `shed-tools` | pending (Stage 5) |
| SSE reconnect throttle (`entity_reconnect_too_fast`) | `shed-entity` | pending (Stage 7) |
| SSE subscriber cap (`entity_stream_full`) + queue-drop-oldest | `shed-entity` | pending (Stage 7) |
| Entity stream disabled flag | `shed-entity` | pending (Stage 7) |
| Speech budget caps (`ENTITY_MAX_UTTERANCES_PER_HOUR/DAY`) | `shed-mind` | pending (Stage 7) |
| Quiet hours | `shed-mind` | pending (Stage 7) |
| Repeat guard (`ENTITY_REPEAT_GUARD_LINES`) | `shed-mind` | pending (Stage 7) |
| Min utterance gap (`ENTITY_MIN_UTTERANCE_GAP_SECONDS`) | `shed-mind` | pending (Stage 7) |
| Outbox TTLs + drain-until-ack | `shed-entity` | pending (Stage 7) |
| The ~20 slow-loop vetoes (`utils/entity_loop.py`) | `shed-mind` (named guard fns) | pending (Stage 7) |
| Grounding checks (market data vs reply claims) | `shed-mind` | pending (Stage 7) |
| Duplicate-event guard (`run_agent` region) | `shed-chat` | pending (Stage 4) |
| Reply completeness/usability recovery | `shed-chat` | pending (Stage 4) |

Workspace gates already live (these guard the repo, not the runtime):

| Gate | Owner | Status |
|---|---|---|
| Frozen assets match `tests/assets.manifest.json` (both directions) | `shed-gates::assets` | **ported** |
| Crate layering + dev-only rules + map completeness | `shed-gates::layers` | **ported** |
| Only `shed-config` reads the environment | `shed-gates::env_read` | **ported** |
| `.env.example` documents exactly the read-set | `shed-gates::settings` | **ported** |
| Guard-catalogue honesty (stale locations, missing entries) | `shed-gates::guards` | pending (first guard) |
