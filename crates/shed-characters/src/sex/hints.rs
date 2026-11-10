//! The name table a sex is read off, and the inference itself
//! (`agent.py:1631`, `agent.py:2237`).

use fancy_regex::Regex;
use serde_json::Value;
use shed_text::text_or_empty;
use std::sync::OnceLock;

use super::core::Sex;

/// The names this app reads a sex off (`agent.py:1631`).
///
/// A name's tokens are scanned in order and the first token found here
/// decides, so the table's own order is irrelevant — a name maps at most one
/// token per lookup. It is kept sorted the way the source is written so a
/// diff against `agent.py` reads straight across.
pub const NAME_SEX_HINTS: &[(&str, Sex)] = &[
    ("abeo", Sex::Male),
    ("adrian", Sex::Male),
    ("amy", Sex::Female),
    ("andrew", Sex::Male),
    ("aria", Sex::Female),
    ("arthur", Sex::Male),
    ("ava", Sex::Female),
    ("banjo", Sex::Male),
    ("baraka", Sex::Male),
    ("clara", Sex::Female),
    ("connor", Sex::Male),
    ("daniel", Sex::Male),
    ("david", Sex::Male),
    ("emma", Sex::Female),
    ("fred", Sex::Male),
    ("george", Sex::Male),
    ("greg", Sex::Male),
    ("hazel", Sex::Female),
    ("jack", Sex::Male),
    ("james", Sex::Male),
    ("jenny", Sex::Female),
    ("kamau", Sex::Male),
    ("karen", Sex::Female),
    ("liam", Sex::Male),
    ("libby", Sex::Female),
    ("maisie", Sex::Female),
    ("mark", Sex::Male),
    ("michelle", Sex::Female),
    ("miguel", Sex::Male),
    ("molly", Sex::Female),
    ("natasha", Sex::Female),
    ("neerja", Sex::Female),
    ("olivia", Sex::Female),
    ("prabhat", Sex::Male),
    ("ryan", Sex::Male),
    ("samantha", Sex::Female),
    ("sibusiso", Sex::Male),
    ("sonia", Sex::Female),
    ("thomas", Sex::Male),
    ("victoria", Sex::Female),
    ("vivienne", Sex::Female),
    ("zira", Sex::Female),
];

/// Python's `dict.get(token)` over the hints.
fn hint_for(token: &str) -> Option<Sex> {
    NAME_SEX_HINTS
        .iter()
        .find(|(name, _)| *name == token)
        .map(|(_, sex)| *sex)
}

/// The compiled patterns `infer_character_sex` uses, in the order the source
/// applies them.
fn pattern(slot: &PatternSlot) -> &'static Regex {
    static VOICE_FEMALE: OnceLock<Regex> = OnceLock::new();
    static VOICE_MALE: OnceLock<Regex> = OnceLock::new();
    static NAME_TOKENS: OnceLock<Regex> = OnceLock::new();
    let (cell, source) = match slot {
        PatternSlot::VoiceFemale => (&VOICE_FEMALE, r"\b(female|feminine|woman|girl)\b"),
        PatternSlot::VoiceMale => (&VOICE_MALE, r"\b(male|masculine|man|boy)\b"),
        PatternSlot::NameTokens => (&NAME_TOKENS, r"[A-Za-z']+"),
    };
    cell.get_or_init(|| {
        Regex::new(source).unwrap_or_else(|error| panic!("{source} must compile: {error}"))
    })
}

enum PatternSlot {
    VoiceFemale,
    VoiceMale,
    NameTokens,
}

/// `infer_character_sex` (`agent.py:2237`): a sex stated by the voice label
/// wins over a name hint; neither says anything, [`Sex::Unstated`].
///
/// The voice patterns are word-matched against the trimmed, lowercased label,
/// so a label reading "Online Female" decides female. The name scan takes the
/// `[A-Za-z']+` tokens in order and returns the first table hit, exactly like
/// Python's loop — "Anna Ryan" is male because no earlier token is in the
/// table, and a name reading "Ryan Amy" is male because "ryan" comes first.
#[must_use]
pub fn infer_character_sex(name: &Value, voice_label: &Value) -> Sex {
    let voice_text = text_or_empty(Some(voice_label)).trim().to_lowercase();
    if pattern(&PatternSlot::VoiceFemale)
        .is_match(&voice_text)
        .unwrap_or(false)
    {
        return Sex::Female;
    }
    if pattern(&PatternSlot::VoiceMale)
        .is_match(&voice_text)
        .unwrap_or(false)
    {
        return Sex::Male;
    }

    let name_text = text_or_empty(Some(name));
    for token in pattern(&PatternSlot::NameTokens).find_iter(&name_text) {
        let Ok(token) = token else { continue };
        if let Some(sex) = hint_for(&token.as_str().to_lowercase()) {
            return sex;
        }
    }
    Sex::Unstated
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_voice_label_s_word_decides_before_any_name() {
        // "greg" says male, but the label outranks it, as in the source.
        assert_eq!(
            infer_character_sex(&json!("Greg"), &json!("Online Female")),
            Sex::Female
        );
        assert_eq!(
            infer_character_sex(&json!("Emma"), &json!("en-GB-Male")),
            Sex::Male
        );
        // Word-matched: a label merely *containing* the letters says nothing.
        assert_eq!(
            infer_character_sex(&json!("Greg"), &json!("femaleish")),
            Sex::Male
        );
    }

    #[test]
    fn name_tokens_are_scanned_in_order() {
        assert_eq!(
            infer_character_sex(&json!("Libby"), &json!("")),
            Sex::Female
        );
        // "Anna" is not in the table; the decision falls through to "ryan".
        assert_eq!(
            infer_character_sex(&json!("Anna Ryan"), &json!("")),
            Sex::Male
        );
        // ...and an earlier hit wins, so the reverse order flips the answer.
        assert_eq!(
            infer_character_sex(&json!("Ryan Amy"), &json!("")),
            Sex::Male
        );
    }

    #[test]
    fn a_name_nothing_recognises_stays_unstated() {
        assert_eq!(infer_character_sex(&json!("Q"), &json!("")), Sex::Unstated);
        assert_eq!(
            infer_character_sex(&json!(null), &json!(null)),
            Sex::Unstated
        );
        // Digits and punctuation split tokens; "5ryan" tokenises to "ryan".
        assert_eq!(infer_character_sex(&json!("5ryan"), &json!("")), Sex::Male);
    }

    #[test]
    fn the_table_matches_the_source_line_for_line() {
        // Pinned so a future edit cannot silently drop or add a name.
        assert_eq!(NAME_SEX_HINTS.len(), 42);
        assert_eq!(hint_for("abeo"), Some(Sex::Male));
        assert_eq!(hint_for("zira"), Some(Sex::Female));
        assert_eq!(hint_for("Zira"), None); // the lookup is pre-lowercased
        assert_eq!(hint_for("gregg"), None);
    }
}
