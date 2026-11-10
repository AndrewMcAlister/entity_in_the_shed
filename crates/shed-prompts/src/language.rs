//! The conversation-language guidance block.
//!
//! `language_display_name` and `build_language_system_prompt` are
//! `utils/prompt.py:128` and `:136`. The language table and the browser
//! header's collapse live in `shed_text::locale` — this module only reads
//! them, exactly as the Python helper imports them from `utils.models`.

use shed_text::{normalize_browser_language, SUPPORTED_LANGUAGES};

/// `language_display_name` (`utils/prompt.py:128`): the English name of the
/// language a browser value collapses to.
///
/// The table is scanned in its declaration order for the entry whose
/// representative locale equals the normalized value — the first match wins,
/// as `for label, primary in SUPPORTED_LANGUAGES.values()` does. An
/// unrepresented locale reads as English, the source's fallback.
#[must_use]
pub fn language_display_name(language_value: &str) -> String {
    let normalized = normalize_browser_language(language_value);
    SUPPORTED_LANGUAGES
        .iter()
        .find(|(_, _, locale)| *locale == normalized)
        .map_or_else(|| "English".to_owned(), |(_, label, _)| (*label).to_owned())
}

/// `build_language_system_prompt` (`utils/prompt.py:136`): the six-line
/// guidance block appended to the system prompt.
///
/// The name comes from the language table and the locale is the normalized
/// value, so an unsupported browser language reads as
/// `English (en-GB)` — the same pair the Python function emits.
#[must_use]
pub fn build_language_system_prompt(preferred_language: &str) -> String {
    let normalized = normalize_browser_language(preferred_language);
    let language_name = language_display_name(&normalized);
    format!(
        "Conversation language guidance:\n\
         - Preferred user language: {language_name} ({normalized}).\n\
         - Understand the user's message in that language when spoken or typed.\n\
         - Reply in {language_name} by default.\n\
         - If the user explicitly asks you to reply in another language, follow that request.\n\
         - Do not mention these language instructions unless the user asks."
    )
}
