//! How good a voice's engine is, read off the words that describe it.
//!
//! Ported from `agent.py:3092` (the tier constants) and `agent.py:3100`
//! (`voice_tier`). The plan (`PLAN-Rust-Clone.md` §3.2) puts those lines with
//! `shed-tts`, but the picker's roster (`agent.py:2637`, `shed-characters`)
//! quotes `voice_tier` for every character entry and the layer map keeps
//! `shed-characters` and `shed-tts` apart — so the rule lives here, next to
//! the voice-depth coercion for exactly the same reason, and `shed-tts`
//! re-exports it as the crate the plan names.

use serde_json::Value;

use crate::value_text::text_or_empty;

/// `VOICE_TIER_MULTILINGUAL` (`agent.py:3092`).
pub const VOICE_TIER_MULTILINGUAL: i64 = 2;
/// `VOICE_TIER_NEURAL` (`agent.py:3093`).
pub const VOICE_TIER_NEURAL: i64 = 1;
/// `VOICE_TIER_LEGACY` (`agent.py:3094`).
pub const VOICE_TIER_LEGACY: i64 = 0;

/// `voice_tier` (`agent.py:3100`): rank a voice by the engine behind it.
///
/// Multilingual is tested before neural: every multilingual ShortName contains
/// both words, so the other order would never reach the better tier. The four
/// fields are joined with a space and lowercased, which is how a description
/// reading "Microsoft (Natural)" earns the neural tier even when the name
/// itself says nothing.
#[must_use]
pub fn voice_tier(voice: &Value) -> i64 {
    let text = ["name", "display_name", "friendly_name", "description"]
        .iter()
        .map(|key| text_or_empty(voice.get(key)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if text.contains("multilingual") || text.contains("multilingue") {
        return VOICE_TIER_MULTILINGUAL;
    }
    if text.contains("neural") || text.contains("natural") {
        return VOICE_TIER_NEURAL;
    }
    VOICE_TIER_LEGACY
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    fn voice(name: &str, extra: &[(&str, Value)]) -> Value {
        let mut map = Map::new();
        map.insert("name".to_owned(), json!(name));
        for (key, value) in extra {
            map.insert((*key).to_owned(), value.clone());
        }
        Value::Object(map)
    }

    #[test]
    fn tiers_prefer_multilingual_then_neural_then_standard() {
        // `voice_tier` checks multilingual/multilingue before neural/natural
        // (`agent.py:3100`), so a name carrying both reads as the better tier.
        assert_eq!(
            voice_tier(&voice("x Multilingual y", &[])),
            VOICE_TIER_MULTILINGUAL
        );
        assert_eq!(
            voice_tier(&voice("Voix Multilingue", &[])),
            VOICE_TIER_MULTILINGUAL
        );
        assert_eq!(
            voice_tier(&voice("MultilingualNeural", &[])),
            VOICE_TIER_MULTILINGUAL
        );
        assert_eq!(
            voice_tier(&voice("en-GB-SoniaNeural", &[])),
            VOICE_TIER_NEURAL
        );
        assert_eq!(
            voice_tier(&voice(
                "x",
                &[("description", json!("Microsoft (Natural) - English (UK)"))]
            )),
            VOICE_TIER_NEURAL
        );
        assert_eq!(
            voice_tier(&voice("Microsoft David Desktop", &[])),
            VOICE_TIER_LEGACY
        );
        assert_eq!(
            voice_tier(&voice("x", &[("description", json!("Online voice"))])),
            VOICE_TIER_LEGACY
        );
        assert_eq!(voice_tier(&voice("Plain Voice", &[])), 0);
    }

    #[test]
    fn a_falsy_field_contributes_nothing_to_the_join() {
        // `str(voice.get(key) or "")`: a null display_name is the empty string,
        // so it cannot smuggle the word "neural" in through `str(None)`.
        assert_eq!(
            voice_tier(&voice("Plain", &[("display_name", Value::Null)])),
            VOICE_TIER_LEGACY
        );
    }
}
