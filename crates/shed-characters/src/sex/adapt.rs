//! The prose rewrite that turns male-authored template text into the chosen
//! sex's (`agent.py:2252–2274`).

use fancy_regex::Regex;
use serde_json::Value;
use shed_text::text_or_empty;
use std::sync::OnceLock;

use super::core::{normalize_character_sex, Sex};

/// The eight substitutions, in the source's dict order (`agent.py:2257`).
///
/// Order is load-bearing the way any sequential `re.sub` chain is: each pass
/// sees the previous pass's output. No replacement here can introduce a word
/// a later pattern matches ("She"/"They" hit none of the remaining `\b...`
/// patterns), so the chain is stable — but the capitalised form of each pair
/// still runs before its lowercase twin, exactly as written.
const REPLACEMENTS: &[(&str, &str, &str)] = &[
    // pattern, female, unknown ("they" set)
    (r"\bHe\b", "She", "They"),
    (r"\bhe\b", "she", "they"),
    (r"\bHim\b", "Her", "Them"),
    (r"\bhim\b", "her", "them"),
    (r"\bHis\b", "Her", "Their"),
    (r"\bhis\b", "her", "their"),
    (r"\bHimself\b", "Herself", "Themself"),
    (r"\bhimself\b", "herself", "themself"),
];

/// One compiled pattern per table row, in row order.
fn compiled() -> &'static [OnceLock<Regex>; 8] {
    // Eight cells, one per `REPLACEMENTS` row. A `Vec<OnceLock>` cannot be
    // `&'static` without leaking, and the table's size is fixed by the source.
    static CELL: OnceLock<[OnceLock<Regex>; 8]> = OnceLock::new();
    CELL.get_or_init(|| [const { OnceLock::new() }; 8])
}

fn pattern(row: usize) -> &'static Regex {
    compiled()[row].get_or_init(|| {
        Regex::new(REPLACEMENTS[row].0)
            .unwrap_or_else(|error| panic!("{} must compile: {error}", REPLACEMENTS[row].0))
    })
}

/// `adapt_character_text_for_sex` (`agent.py:2252`).
///
/// `normalize_character_sex(sex) or "male"`: an unstated sex is *treated as
/// male*, i.e. the text keeps the form it was authored in — the templates are
/// male-authored, so male is the identity, not a rewrite target. Only female
/// and the stated-unknown set rewrite. A sex given as a non-string (a country
/// file authoring `"sex": 0`) normalises through the same `str(value or "")`
/// the source applies.
#[must_use]
pub fn adapt_character_text_for_sex(text: &Value, sex: &Value) -> String {
    // `str(text or "")` — every falsy value, not just null: `False`, `0`,
    // `""`, `[]` and `{}` all start from the empty string too.
    let body = text_or_empty(Some(text));
    let normalized = normalize_character_sex(sex);
    match normalized {
        Sex::Male | Sex::Unstated => body,
        Sex::Female | Sex::Unknown => {
            let female = normalized == Sex::Female;
            let mut adapted = body;
            for (row, (_, replacement_female, replacement_unknown)) in
                REPLACEMENTS.iter().enumerate()
            {
                let replacement = if female {
                    *replacement_female
                } else {
                    *replacement_unknown
                };
                adapted = replace_all(&adapted, pattern(row), replacement);
            }
            adapted
        }
    }
}

/// `re.sub` with a literal replacement: every non-overlapping match, left to
/// right. fancy-regex's `replace_all` takes a closure over captures, so this
/// splices the pieces together by hand rather than escaping `$` rules. None
/// of the eight patterns can match empty, so every match advances the cursor.
fn replace_all(text: &str, regex: &Regex, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut cursor = 0;
    while let Ok(Some(found)) = regex.find(&text[cursor..]) {
        let start = cursor + found.start();
        let end = cursor + found.end();
        out.push_str(&text[last..start]);
        out.push_str(replacement);
        last = end;
        cursor = end;
    }
    out.push_str(&text[last..]);
    out
}

/// `adapt_character_list_for_sex` (`agent.py:2273`).
///
/// Python maps every item through `adapt_character_text_for_sex`, whose first
/// act is `str(text or "")` — so a non-string item comes back as its `str()`
/// spelling even when nothing is rewritten, and `5` becomes `"5"`. Mirrored
/// exactly.
#[must_use]
pub fn adapt_character_list_for_sex(values: &[Value], sex: &Value) -> Vec<Value> {
    values
        .iter()
        .map(|item| Value::String(adapt_character_text_for_sex(item, sex)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn male_and_unstated_keep_the_authored_prose() {
        let text = "He told his own tale, and he enjoyed himself.";
        for sex in [json!("male"), json!(""), json!(null), json!("mr")] {
            assert_eq!(adapt_character_text_for_sex(&json!(text), &sex), text);
        }
    }

    #[test]
    fn female_rewrites_every_gendered_word() {
        assert_eq!(
            adapt_character_text_for_sex(
                &json!("He spent his youth above the harbour. He himself heard him."),
                &json!("female")
            ),
            "She spent her youth above the harbour. She herself heard her."
        );
    }

    #[test]
    fn the_unknown_set_gets_the_neutral_words() {
        // The source rewrites pronouns only; "was" stays "was" even beside
        // "they", because verb agreement is not in the eight patterns.
        assert_eq!(
            adapt_character_text_for_sex(
                &json!("He told his tale; he was not himself."),
                &json!("non-binary")
            ),
            "They told their tale; they was not themself."
        );
    }

    #[test]
    fn word_boundaries_leave_the_bigger_words_alone() {
        // "history"/"himself" inside longer words, and "the" with a letter
        // glued on each side, are not the pronoun.
        assert_eq!(
            adapt_character_text_for_sex(&json!("the history of shame"), &json!("female")),
            "the history of shame"
        );
        // ...but the apostrophe *is* a boundary, so the source rewrites the
        // "he" of "he'll" — pinned here because it is easy to get wrong.
        assert_eq!(
            adapt_character_text_for_sex(&json!("he'll manage"), &json!("female")),
            "she'll manage"
        );
    }

    #[test]
    fn a_non_string_text_is_stringified_not_dropped() {
        assert_eq!(adapt_character_text_for_sex(&json!(5), &json!("male")), "5");
        assert_eq!(
            adapt_character_text_for_sex(&json!(false), &json!("male")),
            ""
        );
        assert_eq!(
            adapt_character_text_for_sex(&json!(null), &json!("female")),
            ""
        );
    }

    #[test]
    fn the_list_rewrites_each_string_and_stringifies_the_rest() {
        let adapted = adapt_character_list_for_sex(
            &[json!("He nodded."), json!(7), json!("plain")],
            &json!("female"),
        );
        assert_eq!(adapted, [json!("She nodded."), json!("7"), json!("plain")]);
    }
}
