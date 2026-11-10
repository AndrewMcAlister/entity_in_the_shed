//! The picker's label rules: human voice names, folder slugs, and the marker
//! that tells two otherwise identical labels apart.
//!
//! Ported from `agent.py:2707` (`voice_human_name`), `agent.py:2730`
//! (`slugify_character_name`), `agent.py:2656` (`character_label_marker`),
//! and `agent.py:2672` (`disambiguate_labels`).

use std::collections::HashMap;
use std::sync::OnceLock;

use fancy_regex::Regex;
use serde_json::Value;

use shed_text::text_or_empty;

use crate::identity::split_character_id;
use shed_text::py_title;

/// One compiled-once pattern; a pattern that fails to compile reads as
/// absent, which no shipped pattern does. The `static` lives per macro
/// expansion, so every call site caches its own.
macro_rules! pattern {
    ($source:expr) => {{
        static SLOT: OnceLock<Option<Regex>> = OnceLock::new();
        SLOT.get_or_init(|| Regex::new($source).ok()).as_ref()
    }};
}

fn replace_all(re: Option<&Regex>, text: &str, with: &str) -> String {
    match re {
        Some(re) => re.replace_all(text, with).into_owned(),
        None => text.to_owned(),
    }
}

/// `voice_human_name` (`agent.py:2707`): derive a human voice name (e.g.
/// "Remy") from a voice's short/display name.
#[must_use]
pub fn voice_human_name(voice: &Value) -> String {
    let short_name = text_or_empty(voice.get("name")).trim().to_owned();
    let display_name = text_or_empty(voice.get("display_name")).trim().to_owned();
    let candidate = if display_name.is_empty() {
        short_name.clone()
    } else {
        display_name.clone()
    };

    // `^[a-z]{2}-[a-z]{2}-` with IGNORECASE: anchored, so re.sub's replace-all
    // can only ever fire once.
    let mut candidate = replace_all(pattern!(r"(?i)^[a-z]{2}-[a-z]{2}-"), &candidate, "");
    // The camel split needs a lookbehind, which RE2 cannot express.
    candidate = replace_all(pattern!(r"(?<=[a-z])(?=[A-Z])"), &candidate, " ");
    candidate = replace_all(
        pattern!(
            r"(?i)\b(multilingual|neural|expressive|online|natural|desktop|voice|speech|microsoft|google)\b"
        ),
        &candidate,
        " ",
    );
    candidate = replace_all(pattern!(r"[-_/()]"), &candidate, " ");
    candidate = replace_all(pattern!(r"\s+"), &candidate, " ");
    let candidate = candidate.trim().to_owned();

    let words: Vec<String> = pattern!(r"[A-Za-z']+")
        .map(|re| {
            re.find_iter(&candidate)
                .filter_map(Result::ok)
                .map(|matched| matched.as_str().to_owned())
                .filter(|word| word.chars().count() > 1)
                .collect()
        })
        .unwrap_or_default();

    match words.first() {
        Some(word) => word.clone(),
        // `short_name or display_name` — both already stripped above.
        None if short_name.is_empty() => display_name,
        None => short_name,
    }
}

/// `slugify_character_name` (`agent.py:2730`): a filesystem-safe slug for a
/// character name; unspellable input still yields "character".
///
/// The Python signature takes any value and spells `str(value or "")`; the
/// clone takes the `&str` its only caller has already read through
/// [`shed_text::text_or_empty`].
#[must_use]
pub fn slugify_character_name(value: &str) -> String {
    let lowered = value.trim().to_lowercase();
    let slug = replace_all(pattern!(r"[^a-z0-9]+"), &lowered, "-");
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "character".to_owned()
    } else {
        slug.to_owned()
    }
}

/// `character_label_marker` (`agent.py:2656`): a short tag telling two
/// otherwise identical labels apart.
///
/// Prefers the voice's script subtag — "iu-Cans-CA-SiqiniqNeural" against
/// "iu-Latn-CA-SiqiniqNeural" differ by script and nothing else — then the
/// disambiguated part of the character's own key.
#[must_use]
pub fn character_label_marker(entry: &Value) -> String {
    let name = text_or_empty(entry.get("name"));
    // `split("-")[1:-1]`: the language head and the trailing "Neural" tail
    // never tell two characters apart, so the scan skips both.
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() > 2 {
        for part in &parts[1..parts.len() - 1] {
            if part.chars().count() == 4 && part.chars().all(char::is_alphabetic) {
                return py_title(part);
            }
        }
    }

    let (_country, key) = split_character_id(&text_or_empty(entry.get("id")));
    // `key[len(character_name):]` slices by Unicode character, which is what
    // counting `chars()` counts.
    let name_len = text_or_empty(entry.get("character_name")).chars().count();
    let suffix: String = key.chars().skip(name_len).collect();
    let suffix = suffix.trim_matches('-');
    if suffix.is_empty() {
        name
    } else {
        py_title(suffix)
    }
}

/// `disambiguate_labels` (`agent.py:2672`): make every label unique, since
/// the picker shows only the label.
///
/// Two characters can share a name and a region — Canada ships Siqiniq in
/// both Inuktitut scripts, neither with a region of its own — so when labels
/// collide each gains the tag that actually tells them apart.
///
/// The counts are taken once over the original labels, exactly like the
/// source: two entries that collide gain markers and could still collide, and
/// the source does not re-check, so neither does this.
pub fn disambiguate_labels(entries: &mut [Value]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in entries.iter() {
        *counts.entry(text_or_empty(entry.get("label"))).or_default() += 1;
    }
    for entry in entries.iter_mut() {
        let label = text_or_empty(entry.get("label"));
        if counts.get(&label).copied().unwrap_or_default() > 1 {
            let marker = character_label_marker(entry);
            if let Some(object) = entry.as_object_mut() {
                object.insert(
                    "label".to_owned(),
                    Value::String(format!("{label} ({marker})")),
                );
            }
        }
    }
}
