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
//! | `normalize_locale` / `SUPPORTED_LANGUAGES` / `normalize_browser_language` | here, in `locale.rs`, because both the model catalog and the voice catalog need them — these three are the part of `utils/models.py` that belongs to text, the rest stays with `shed-models` (§3.2) |
//! | the `str(value or "")` reading idiom, and the `repr()`/`!r` diagnostic idiom (`py_repr`/`py_float_repr`) | here, in `value_text.rs`, because `utils/characters.py` and the voice half of `agent.py` both read authored data through it — and the character validators name bad voices and pitches through `!r` — and the layer map forbids those crates depending on each other |
//! | voice-depth coercion (`clamp`/`normalize_voice_depth`/`coerce_voice_depth` from `agent.py`) | here, in `voice_depth.rs`, for the same reason: `shed-characters` validates authored pitches and `shed-tts` synthesises with them through one rule |
//! | voice-tier classification (`VOICE_TIER_*`/`voice_tier` from `agent.py`) | here, in `voice_tier.rs`, for the same reason: the picker's roster mints a tier for every character entry and `/api/voices` scores voices with it |
//! | `str.title()` / `str.splitlines()` spellings | here, in `py_text.rs`, because `shed-characters` (labels, profile lines) and `shed-prompts` (template variables, frontmatter) both quote them and may not depend on each other |
//! | the rest of the file | the stage that first calls it (`PLAN-Rust-Clone.md` §8.1) |

mod locale;
mod py_text;
mod snippet;
mod value_text;
mod voice_depth;
mod voice_tier;

pub use locale::{normalize_browser_language, normalize_locale, SUPPORTED_LANGUAGES};
pub use py_text::{py_splitlines, py_title};
pub use snippet::{safe_body_snippet, MAX_SNIPPET_CHARS};
pub use value_text::{py_float_repr, py_repr, py_str, text_or_empty};
pub use voice_depth::{coerce_f64, coerce_voice_depth, normalize_voice_depth};
pub use voice_tier::{voice_tier, VOICE_TIER_LEGACY, VOICE_TIER_MULTILINGUAL, VOICE_TIER_NEURAL};
