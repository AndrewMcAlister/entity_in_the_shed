//! The size hint written inside a model's name.
//!
//! Ported from `utils/models.py:10` (`parse_model_size_billions`), which is how the app knows
//! which model is the smallest one it has: a discovery payload rarely carries a byte count, but a
//! name like `llama-3.1-8b` says it out loud.

use std::sync::OnceLock;

use fancy_regex::Regex;

/// `utils/models.py:11`'s pattern, verbatim. The `(?i)` is `flags=re.IGNORECASE`.
///
/// The negative lookbehind is the reason this crate uses `fancy-regex`: `mistral-7b` matches, but
/// the `3` in `qwen3.8` and the `3` in `iq3_xxs` do not, because a letter sits in front of them.
/// The tail `(?:\b|[^a-z0-9])` is the other half of the same rule: `8b` is a size, `8bit` is a
/// word that happens to start with one.
const NAME_SIZE_PATTERN: &str = r"(?i)(?<![a-zA-Z])(\d+(?:\.\d+)?)b(?:\b|[^a-z0-9])";

/// The compiled pattern. Python compiles once and caches it in `re`; a `OnceLock` is the same
/// thing, and the scan runs once per model per discovery.
fn name_size_pattern() -> &'static Regex {
    static COMPILED: OnceLock<Regex> = OnceLock::new();
    COMPILED.get_or_init(|| {
        Regex::new(NAME_SIZE_PATTERN)
            .unwrap_or_else(|error| panic!("{NAME_SIZE_PATTERN} must compile: {error}"))
    })
}

/// The billions in a model name, or infinity when the name does not say.
///
/// Infinity is the source's answer (`utils/models.py:13`) and it is load-bearing: the discovery
/// sort puts unknown sizes last, so a model the app cannot measure never takes the place of one it
/// can. A name that is not a string is the empty string here, which is `str(model_name or "")`
/// there.
pub fn parse_model_size_billions(model_name: &str) -> f64 {
    let Ok(Some(captures)) = name_size_pattern().captures(model_name) else {
        return f64::INFINITY;
    };

    captures[1].parse::<f64>().unwrap_or(f64::INFINITY) // 15-18: the `except ValueError`
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_read_out_of_the_names_that_spell_it() {
        assert_eq!(parse_model_size_billions("glm-4 9b"), 9.0);
        assert_eq!(parse_model_size_billions("llama-3.1-8b"), 8.0);
        assert_eq!(parse_model_size_billions("qwen2.5-7b-instruct"), 7.0);
        assert_eq!(parse_model_size_billions("mistral-7b-v0.1"), 7.0);
    }

    #[test]
    fn a_name_that_does_not_say_its_size_is_infinite() {
        assert_eq!(parse_model_size_billions("gpt-2"), f64::INFINITY);
        assert_eq!(parse_model_size_billions(""), f64::INFINITY);
    }

    #[test]
    fn a_digit_preceded_by_a_letter_is_not_a_size() {
        // The operator's own engine answers `/v1/models` with this name, and the app has no idea
        // how big it is from the name alone: both digit runs have a letter in front of them.
        assert_eq!(
            parse_model_size_billions("qwen3.8-flash-next-iq3_xxs"),
            f64::INFINITY
        );
    }

    #[test]
    fn a_size_that_is_only_the_start_of_a_word_is_not_a_size() {
        assert_eq!(parse_model_size_billions("model-8bit"), f64::INFINITY);
        assert_eq!(
            parse_model_size_billions("model-8bit quant"),
            f64::INFINITY,
            "what follows does not matter: the tail rule fails at the `i`"
        );

        // The scan does keep looking after a rejected run, so a real size further along is found.
        assert_eq!(parse_model_size_billions("8bit-8b"), 8.0);
    }

    #[test]
    fn a_fraction_is_kept() {
        assert_eq!(parse_model_size_billions("mixtral-22.5b"), 22.5);
    }

    #[test]
    fn the_largest_number_in_the_name_wins_because_the_scan_is_leftmost() {
        // `re.search` and `Regex::captures` both report the first match, not the biggest: the
        // sort that uses this value only needs a stable order, not the truth about the model.
        assert_eq!(parse_model_size_billions("70b-8b"), 70.0);
    }
}
