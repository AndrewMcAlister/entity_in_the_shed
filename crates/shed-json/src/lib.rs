//! JSON safety, strict response parsing, token-byte helpers.
//!
//! Ported from `utils/json_util.py`. Only the helper a stage actually calls is
//! carried (`How_To_Code` principle 16), so this crate grows one function at a
//! time:
//!
//! | `utils/json_util.py` | Where it goes |
//! |---|---|
//! | `parse_json_response_strict` | here, in `response.rs`, because `agent.py:1559` reads `GET {LM_ENGINE_HOST}/v1/models` through it during Stage 2's discovery |
//! | `make_json_safe` | Stage 5, whose only production callers are `utils/mcp.py:88`, `102`, `119` |
//! | `encode_token_bytes`, `decode_token_bytes` | nowhere: `agent.py:106-107` imports them and nothing in the source calls them any more, so `TestTokenBytes` is not ported (`PLAN-Rust-Clone.md` §9) |

mod response;

pub use response::parse_json_response_strict;
pub use response::JsonError;
