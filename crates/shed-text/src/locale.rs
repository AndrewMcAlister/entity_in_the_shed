//! Locale and browser-language normalisation.
//!
//! Ported from `utils/models.py:50` (`normalize_locale`), `utils/models.py:64`
//! (`SUPPORTED_LANGUAGES`) and `utils/models.py:88`
//! (`normalize_browser_language`). Plan §3.3 puts locale helpers in this crate
//! because both the model catalog (which browser name a model was picked under)
//! and the voice catalog (which locale a voice speaks) need the same rules, and
//! §3.2 records that `utils/models.py` splits across `shed-models` and here.

/// `normalize_locale` (`utils/models.py:50`): underscore to hyphen, drop empty
/// segments, lower-case the language, upper-case the region.
///
/// Only the first two segments survive: `pt_BR_1` normalises to `pt-BR`, the
/// same answer Python's `parts[0]`/`parts[1]` give.
#[must_use]
pub fn normalize_locale(value: &str) -> String {
    let text = value.trim().replace('_', "-");
    let parts: Vec<&str> = text.split('-').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [] => String::new(),
        [language] => language.to_lowercase(),
        [language, region, ..] => format!("{}-{}", language.to_lowercase(), region.to_uppercase()),
    }
}

/// `SUPPORTED_LANGUAGES` (`utils/models.py:64`): language code -> (English
/// name, representative locale). The representative locale is what a browser
/// `Accept-Language` head collapses to.
pub const SUPPORTED_LANGUAGES: [(&str, &str, &str); 20] = [
    ("en", "English", "en-GB"),
    ("fr", "French", "fr-FR"),
    ("ja", "Japanese", "ja-JP"),
    ("es", "Spanish", "es-ES"),
    ("de", "German", "de-DE"),
    ("it", "Italian", "it-IT"),
    ("pt", "Portuguese", "pt-BR"),
    ("zh", "Chinese (Mandarin)", "zh-CN"),
    ("ru", "Russian", "ru-RU"),
    ("ar", "Arabic", "ar-SA"),
    ("hi", "Hindi", "hi-IN"),
    ("ko", "Korean", "ko-KR"),
    ("nl", "Dutch", "nl-NL"),
    ("pl", "Polish", "pl-PL"),
    ("tr", "Turkish", "tr-TR"),
    ("ur", "Urdu", "ur-PK"),
    ("sv", "Swedish", "sv-SE"),
    ("nb", "Norwegian", "nb-NO"),
    ("da", "Danish", "da-DK"),
    ("hr", "Croatian", "hr-HR"),
];

/// The representative locale for a language code, mirroring
/// `SUPPORTED_LANGUAGES[language][1]`.
fn supported_locale(language: &str) -> Option<&'static str> {
    SUPPORTED_LANGUAGES
        .iter()
        .find(|(code, _, _)| *code == language)
        .map(|(_, _, locale)| *locale)
}

/// `normalize_browser_language` (`utils/models.py:88`): a browser language
/// collapses to its supported representative, anything else to `en-GB`.
#[must_use]
pub fn normalize_browser_language(language_value: &str) -> String {
    let normalized = normalize_locale(language_value);
    let language = normalized.split('-').next().unwrap_or("");
    supported_locale(language).unwrap_or("en-GB").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_is_normalised_to_language_and_region() {
        assert_eq!(normalize_locale("en_us"), "en-US");
        assert_eq!(normalize_locale("PT-br"), "pt-BR");
        assert_eq!(normalize_locale(" ja_JP "), "ja-JP");
    }

    #[test]
    fn a_bare_language_keeps_no_region() {
        assert_eq!(normalize_locale("fr"), "fr");
        assert_eq!(normalize_locale("DE"), "de");
    }

    #[test]
    fn empty_and_punctuation_only_values_normalise_to_empty() {
        assert_eq!(normalize_locale(""), "");
        assert_eq!(normalize_locale("   "), "");
        assert_eq!(normalize_locale("---"), "");
        assert_eq!(normalize_locale("_"), "");
    }

    #[test]
    fn only_the_first_two_segments_survive() {
        assert_eq!(normalize_locale("pt_BR_1"), "pt-BR");
        assert_eq!(normalize_locale("es-MX-extra"), "es-MX");
    }

    #[test]
    fn a_supported_browser_language_collapses_to_its_representative() {
        assert_eq!(normalize_browser_language("en-US"), "en-GB");
        assert_eq!(normalize_browser_language("fr-CA"), "fr-FR");
        assert_eq!(normalize_browser_language("pt_BR"), "pt-BR");
        assert_eq!(normalize_browser_language("zh-TW"), "zh-CN");
        // A bare language code is already a language.
        assert_eq!(normalize_browser_language("ja"), "ja-JP");
    }

    #[test]
    fn an_unsupported_or_missing_language_falls_back_to_british_english() {
        assert_eq!(normalize_browser_language("sw"), "en-GB");
        assert_eq!(normalize_browser_language("fil-PH"), "en-GB");
        assert_eq!(normalize_browser_language(""), "en-GB");
        assert_eq!(normalize_browser_language("---"), "en-GB");
    }

    #[test]
    fn every_supported_language_maps_to_a_locale_that_starts_with_its_code() {
        for (code, _name, locale) in SUPPORTED_LANGUAGES {
            assert!(
                locale.starts_with(&format!("{code}-")),
                "{code} should be the language part of {locale}"
            );
            assert_eq!(normalize_browser_language(code), locale);
        }
    }
}
