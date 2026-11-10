//! The voice-tone enumeration block.
//!
//! `_build_tone_system_prompt` is `utils/prompt.py:149`. The settings map it
//! enumerates is `TONE_TTS_SETTINGS` (`agent.py:619`), which lives in
//! `agent.py` rather than in `utils/prompt.py`, so the Python helper takes it
//! as a parameter — and so does this one. The stage that first needs the map
//! itself (the chat prompt, then the TTS pipeline) passes it in.

use std::collections::BTreeMap;

use serde_json::Value;
use shed_text::text_or_empty;

/// `_build_tone_system_prompt` (`utils/prompt.py:149`): one line per tone,
/// sorted by name, with the tone's description when it has one.
///
/// A tone whose `description` is missing or blank contributes a bare
/// `- {name}`, which is what keeps a settings entry that is pure rate/pitch
/// data advertiseable without prose. `BTreeMap` iteration is the source's
/// `sorted(tone_settings.keys())`.
#[must_use]
pub fn build_tone_system_prompt(tone_settings: &BTreeMap<String, Value>) -> String {
    let mut lines = vec!["Available voice tone tags:".to_owned()];
    for (name, tone) in tone_settings {
        let description = text_or_empty(tone.get("description")).trim().to_owned();
        if description.is_empty() {
            lines.push(format!("- {name}"));
        } else {
            lines.push(format!("- {name}: {description}"));
        }
    }
    lines.join("\n")
}
