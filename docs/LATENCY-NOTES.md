# Speech-path latency — measurements and their consequences

Measured on this box (2026-11-12). Moved here from the tail of `TODO/PLAN-Rust-Clone.md`,
where a paste from another conversation had left it appended after §12 in the wrong format.
The measurements are real and Stage 6 (`shed-tts`) needs them; the plan's §8 Stage 6 row
points here.

## Where the wall-clock time goes today

| Stage | Measured |
|---|---|
| Waiting for a request | ~5 s — **before** the model receives anything |
| Prefill (~1.1k prompt) | 1.9 s at 563–593 tok/s |
| Generation (thinking + reply) | ~0.4–1 s |

So the model contributes ~2.3 s after a request lands. The ~5 s the user hears is in the
**voice pipeline**, not in generation.

## Endpoint latency from this machine

| Endpoint | Round trip |
|---|---|
| `speech.platform.bing.com` (Edge voice) | 17–33 ms |
| `australiaeast.api.cognitive.microsoft.com` | 25–46 ms |
| `eastus.api.cognitive.microsoft.com` | 214–231 ms |

A token fetch plus chunked audio upload plus finalise is many round trips. An Azure Speech
resource in East US therefore pays ~220 ms per hop, which is easily seconds in total. Moving
the resource to `australiaeast` is a configuration change with no code. If ~5 s remains with a
nearby region, the cause is endpointing/batching rather than distance.

## What this implies for the Rust build (Stage 6)

1. **Stream audio and finalise on silence** (~300–500 ms endpoint) instead of stop → upload →
   wait for one batch transcript.
2. **Start synthesis on the first sentence**, not after the whole reply. The reply is short
   (~350 tokens), so the model produces text faster than the speech layer consumes it.
3. **A local voice** (whisper.cpp / Piper) removes the cloud hop entirely if depending on a
   region is unacceptable.

None of the three changes the wire contract (§4.4); they change when the work starts, not what
the client receives.
