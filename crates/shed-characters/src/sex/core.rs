//! The canonical sex value and its pronoun set (`agent.py:2196–2234`).

use serde_json::{json, Map, Value};

use shed_text::text_or_empty;

/// The canonical sex values, plus the source's `""` for "not stated".
///
/// Python returns a string here and each caller decides what an empty string
/// means: the profile builder treats it as "ask the voice, then the name",
/// while [`super::adapt::adapt_character_text_for_sex`] folds it to male.
/// An enum makes the fourth case explicit instead of letting it hide as `""`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sex {
    Male,
    Female,
    /// `normalize_character_sex`'s third value: stated as unknown/neutral.
    Unknown,
    /// Nothing recognisable was stated — Python's `""`.
    Unstated,
}

impl Sex {
    /// The word a profile writes this sex as — Python's `""` for
    /// [`Sex::Unstated`], which is exactly what makes it falsy in an `or`
    /// chain like `voice_sex or template_sex or ...`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
            Self::Unknown => "unknown",
            Self::Unstated => "",
        }
    }
}

/// `normalize_character_sex` (`agent.py:2196`), over the `str(value or "")`
/// the callers spell it with.
///
/// The alias sets are the source's exactly: `masculine`/`feminine` are words
/// a voice label's `gender` field can carry, and the unknown set is what a
/// country file authors when a character is deliberately not gendered.
#[must_use]
pub fn normalize_character_sex(value: &Value) -> Sex {
    match text_or_empty(Some(value)).trim().to_lowercase().as_str() {
        "male" | "masculine" | "m" | "man" | "boy" => Sex::Male,
        "female" | "feminine" | "f" | "woman" | "girl" => Sex::Female,
        "unknown" | "unspecified" | "neutral" | "other" | "nonbinary" | "non-binary" => {
            Sex::Unknown
        }
        _ => Sex::Unstated,
    }
}

/// One pronoun set: the six fields the persona is written with. A `BTreeMap`
/// behind `serde_json::Map`, which is the sorted key order the HTTP responses
/// are byte-compared in.
pub type Pronouns = Map<String, Value>;

fn set(
    sex: &str,
    subject: &str,
    object: &str,
    possessive_adjective: &str,
    possessive_pronoun: &str,
    reflexive: &str,
) -> Pronouns {
    let mut map = Pronouns::new();
    map.insert("sex".to_owned(), json!(sex));
    map.insert("subject_pronoun".to_owned(), json!(subject));
    map.insert("object_pronoun".to_owned(), json!(object));
    map.insert(
        "possessive_adjective".to_owned(),
        json!(possessive_adjective),
    );
    map.insert("possessive_pronoun".to_owned(), json!(possessive_pronoun));
    map.insert("reflexive_pronoun".to_owned(), json!(reflexive));
    map
}

/// `pronouns_for_character_sex` (`agent.py:2207`): `normalize(...) or
/// "unknown"`, so an unstated sex gets the neutral set.
///
/// The neutral reflexive is "themself", not "themselves" — that is what the
/// source writes and what the persona prose quotes back.
#[must_use]
pub fn pronouns_for_character_sex(sex: Sex) -> Pronouns {
    match sex {
        Sex::Male => set("male", "he", "him", "his", "his", "himself"),
        Sex::Female => set("female", "she", "her", "her", "hers", "herself"),
        Sex::Unknown | Sex::Unstated => {
            set("unknown", "they", "them", "their", "theirs", "themself")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_alias_sets_are_the_source_s() {
        for alias in ["Male", " M ", "man", "BOY", "masculine"] {
            assert_eq!(normalize_character_sex(&json!(alias)), Sex::Male);
        }
        for alias in ["Female", " f ", "woman", "GIRL", "feminine"] {
            assert_eq!(normalize_character_sex(&json!(alias)), Sex::Female);
        }
        for alias in [
            "unknown",
            "Unspecified",
            "neutral",
            "other",
            "nonbinary",
            "non-binary",
        ] {
            assert_eq!(normalize_character_sex(&json!(alias)), Sex::Unknown);
        }
    }

    #[test]
    fn anything_else_is_unstated_including_the_falsy_values() {
        // `str(value or "")` collapses None/False/0/[] to "" before the match,
        // and "mr"/"mister" are *not* in the source's male set despite reading male.
        for blank in [json!(null), json!(false), json!(0), json!(""), json!([])] {
            assert_eq!(normalize_character_sex(&blank), Sex::Unstated);
        }
        assert_eq!(normalize_character_sex(&json!("mr")), Sex::Unstated);
    }

    #[test]
    fn each_sex_gets_exactly_the_source_s_six_fields() {
        // serde_json's Map is a BTreeMap without `preserve_order`, and that is
        // the order the HTTP layer wants anyway (sorted keys, byte-parity).
        let male = pronouns_for_character_sex(Sex::Male);
        let keys: Vec<&str> = male.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "object_pronoun",
                "possessive_adjective",
                "possessive_pronoun",
                "reflexive_pronoun",
                "sex",
                "subject_pronoun",
            ]
        );
        assert_eq!(male["subject_pronoun"], json!("he"));
        assert_eq!(male["reflexive_pronoun"], json!("himself"));

        let female = pronouns_for_character_sex(Sex::Female);
        assert_eq!(female["possessive_pronoun"], json!("hers"));

        // Stated-unknown and unstated share the neutral set, and its `sex`
        // field reads "unknown" for both — the source labels the *set*.
        for sex in [Sex::Unknown, Sex::Unstated] {
            let neutral = pronouns_for_character_sex(sex);
            assert_eq!(neutral["sex"], json!("unknown"));
            assert_eq!(neutral["subject_pronoun"], json!("they"));
            assert_eq!(neutral["reflexive_pronoun"], json!("themself"));
        }
    }
}
