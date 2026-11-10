//! The voice catalog: which voices the app believes it can speak with, how
//! good each one is, and which one a request gets.
//!
//! Ported from the voice half of `agent.py` (plan §3.2 assigns `agent.py:3086`
//! to here). Only the helper a stage actually calls is carried (`How_To_Code`
//! principle 16), so this crate grows one function at a time, in the stage
//! that first needs it:
//!
//! | `agent.py` | Where it goes |
//! |---|---|
//! | `voice_matches_language`, `voice_tier`, `accent_preference`, `voice_preference_score`, `list_available_voices`, `choose_tts_voice`, `clamp`, `normalize_voice_depth` | here, in `catalog.rs`, because Stage 3 serves `/api/voices` and filters the character rosters through the same catalog |
//! | `list_edge_voices`, `list_windows_voices` | Stage 6, which replaces the static snapshot with the live Edge fetch and the Windows SAPI lookup (plan §8 Stage 3 divergence row in `docs/PARITY.md`) |
//! | the rest of the TTS file | the stage that first calls it |

mod catalog;

pub use catalog::{
    choose_tts_voice, coerce_f64, coerce_voice_depth, list_available_voices, normalize_voice_depth,
    voice_matches_language, voice_preference_score, voice_tier, VOICE_TIER_LEGACY,
    VOICE_TIER_MULTILINGUAL, VOICE_TIER_NEURAL, VOICE_TIER_WEIGHT,
};
