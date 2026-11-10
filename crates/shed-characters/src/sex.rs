//! Sex: the canonical value, its pronoun set, the inference from a voice
//! label or a name, and the prose rewrite that keeps male-authored template
//! text speak as the chosen character's sex.
//!
//! Ported from `agent.py:2196–2275` (`normalize_character_sex`,
//! `pronouns_for_character_sex`, `infer_character_sex`,
//! `adapt_character_text_for_sex`, `adapt_character_list_for_sex`).
//!
//! | Python | Here |
//! |---|---|
//! | `normalize_character_sex`, `pronouns_for_character_sex` | [`core`] |
//! | `NAME_SEX_HINTS` (`agent.py:1631`), `infer_character_sex` | [`hints`] |
//! | `adapt_character_text_for_sex`, `adapt_character_list_for_sex` | [`adapt`] |

pub mod adapt;
pub mod core;
pub mod hints;

pub use adapt::{adapt_character_list_for_sex, adapt_character_text_for_sex};
pub use core::{normalize_character_sex, pronouns_for_character_sex, Pronouns, Sex};
pub use hints::infer_character_sex;
