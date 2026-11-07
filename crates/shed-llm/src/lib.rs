//! The transport to LM Studio: how a request is made, how a transient failure is retried, how a
//! streamed reply is read, and how the model list is fetched.
//!
//! Ported from `agent.py`'s transport block. Only what a stage has a caller for is written
//! (`How_To_Code` principle 16), which is why this crate is four files wide at Stage 2 and grows
//! with the stages that send a chat request.
//!
//! | `agent.py` | Where it goes |
//! |---|---|
//! | `call_lm_http_with_retry` (1038), `_is_retryable_lm_failure` (1030) | here, in `retry.rs` |
//! | `discover_lm_studio_models` (1529), its HTTP half (1535-1562) | here, in `discovery.rs`; its catalog half (1564-1629) is `shed_models::apply_discovered_payload`, because `LAYERS` lets `shed-llm` reach `shed-models` and never the reverse (§3.3) |
//! | `current_request_id` (973) | the generated half (982-983) is here, in `request_id.rs`; the `X-Request-ID` half (978, 1015, 1023) is Stage 3's, with the routes that have a request to read one off |
//! | `LmStreamUnavailable` (1073), `read_lm_chat_stream` (1081), `call_lm_http_stream_chat` (1153), `_stream_rejection` (1213), `_lm_stream_backoff` (1223) | here, in `stream.rs`: §8 names the streaming reader as a Stage 2 deliverable and §9 ports `test_lm_streaming` to this crate at Stage 2, so the parity suite is its caller even though the first production caller, `call_lm_engine` (6618, 7016), arrives at Stage 4 |
//! | `call_lm_engine` (6618), `MODEL_PATHS`' readers (6835-6858), `LM_ENGINE_KEEP_ALIVE` and the sampler settings | Stage 4 |
//! | `find_local_lmstudio_model_match` (6341), `get_local_candidate_for` (6377), `LOCAL_MODEL_MAP` (354-370), `AI_MODEL_LOCAL_OVERRIDE` / `AI_MODEL_LOCAL_OVERRIDES` (376-388) | nowhere: the cluster calls only itself, and `docs/PARITY.md`, "Surfaces the clone does not carry", records the no-caller evidence |
//!
//! ## The dynamic value that passes through
//!
//! `PLAN-Rust-Clone.md` §3.3 bans `serde_json::Value` outside `shed-json` and the MCP argument
//! layer. This crate is the corridor, not the exception: `fetch_discovery_payload` hands the value
//! `shed-json` produced straight to `shed-models`, which is where the exception is written and where
//! the value is consumed, and `call_lm_http_stream_chat` serialises a chat body it never reads —
//! Stage 4 assembles it, the engine consumes it. Nothing here stores one, and nothing here parses
//! one, unless it is an SSE event on its way to becoming a delta.

mod discovery;
mod request_id;
mod retry;
mod stream;

pub use discovery::discover_lm_studio_models;
pub use request_id::new_request_id;
pub use retry::{LmRequestError, LmTransport};
pub use stream::{
    call_lm_http_stream_chat, read_lm_chat_stream, LmStreamUnavailable, StreamFailure, StreamRead,
    StreamTimeout,
};
