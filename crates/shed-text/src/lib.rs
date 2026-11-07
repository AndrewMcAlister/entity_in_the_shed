//! Text helpers: token estimation, tone-tag and markup normalisation,
//! vocalisation respelling, bracket segmentation.
//!
//! Ported from `utils/text.py`. Only the helper a stage actually calls is
//! carried (`How_To_Code` principle 16), so this crate grows one function at
//! a time, in the stage that first needs it:
//!
//! | `utils/text.py` | Where it goes |
//! |---|---|
//! | `_safe_body_snippet` | here, in `snippet.rs`, because Stage 2's strict response parser reports the body it could not parse |
//! | the rest of the file | the stage that first calls it (`PLAN-Rust-Clone.md` §8.1) |

mod snippet;

pub use snippet::{safe_body_snippet, MAX_SNIPPET_CHARS};
