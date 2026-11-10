//! The roster one country offers the picker, and the fields each entry
//! carries.
//!
//! Ported from `agent.py:2574` (`country_roster`), `agent.py:2522`
//! (`character_details`), `agent.py:2545` (`character_text_lines`), and
//! `agent.py:2561` (`character_dedupe_key`).
//!
//! `country_roster`'s `voices=None` fallback — Python reaching for
//! `list_available_voices()` and warning when that lookup fails
//! (`agent.py:2583-2588`) — is the caller's: the layer map keeps this crate
//! away from `shed-tts`, so the route fetches the catalog and hands it in.
//! An empty slice is Python's falsy `voices` after a failed lookup: no "not
//! shipped" warnings, and no entries either, because no character can
//! resolve a voice.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use tracing::warn;

use shed_text::{coerce_voice_depth, py_str, text_or_empty, voice_tier};

use crate::catalog::{validate_character_voices, CharacterCatalog};
use crate::identity::character_id;
use crate::picker::labels::{disambiguate_labels, voice_human_name};
use shed_text::py_splitlines;

impl CharacterCatalog {
    /// `country_roster` (`agent.py:2574`): the picker entries for one country.
    ///
    /// A country offers exactly the characters its file declares, minus any
    /// whose voice the live catalog does not actually ship, so a profile for
    /// a voice that cannot be synthesised never reaches the picker. Every
    /// character left out is named in a warning: leaving it out is the
    /// correct behaviour, but a silent drop cannot be told apart from the
    /// character having been deleted.
    #[must_use]
    pub fn country_roster(&self, country: &str, voices: &[Value]) -> Vec<Value> {
        let definitions = self.character_keys(country);

        // Only a catalog we actually hold can say a voice is missing. An
        // empty one means the lookup failed — a different fault, already
        // reported by whoever failed it — and calling it "not shipped" would
        // name a voice as absent on no evidence (`agent.py:2592-2596`).
        if !voices.is_empty() {
            let problems = validate_character_voices(&definitions, voices);
            if !problems.is_empty() {
                warn!(
                    "character voices not shipped in {country}:\n{}",
                    problems.join("\n")
                );
            }
        }

        let by_name = voices_by_name(voices);
        let mut entries = Vec::new();
        for (key, definition) in &definitions {
            if let Some(entry) = roster_entry(country, key, definition, &by_name) {
                entries.push(entry);
            }
        }
        disambiguate_labels(&mut entries);
        entries
    }
}

/// `by_name` (`agent.py:2603-2607`): the first voice under each ShortName.
fn voices_by_name(voices: &[Value]) -> HashMap<String, &Value> {
    let mut by_name = HashMap::new();
    for voice in voices {
        let short_name = text_or_empty(voice.get("name")).trim().to_owned();
        if !short_name.is_empty() && !by_name.contains_key(&short_name) {
            by_name.insert(short_name, voice);
        }
    }
    by_name
}

/// One entry of the roster loop (`agent.py:2610-2651`); `None` is the
/// `voice is None: continue` arm — a voice the catalog does not ship.
fn roster_entry(
    country: &str,
    key: &str,
    definition: &Value,
    by_name: &HashMap<String, &Value>,
) -> Option<Value> {
    let voice_name = text_or_empty(definition.get("voice")).trim().to_owned();
    let voice = *by_name.get(&voice_name)?;

    let authored = text_or_empty(definition.get("name")).trim().to_owned();
    let character_name = if authored.is_empty() {
        voice_human_name(voice)
    } else {
        authored
    };
    let region = text_or_empty(definition.get("region")).trim().to_owned();

    let mut entry = Map::new();
    entry.insert("id".to_owned(), json!(character_id(country, key)));
    // "name" stays the voice ShortName so the client can hand it straight
    // back to /api/speak for synthesis (`agent.py:2621-2622`).
    entry.insert("name".to_owned(), json!(voice_name.clone()));
    entry.insert(
        "display_name".to_owned(),
        json!(or_voice_name(voice.get("display_name"), &voice_name).trim()),
    );
    entry.insert("character_name".to_owned(), json!(character_name.clone()));
    // The country is already chosen from the picker above, so the label
    // spends itself on the region instead of repeating it
    // (`agent.py:2626-2627`).
    entry.insert(
        "label".to_owned(),
        json!(if region.is_empty() {
            character_name.clone()
        } else {
            format!("{character_name} - {region}")
        }),
    );
    entry.insert(
        "provider".to_owned(),
        json!(py_or_text(
            definition.get("provider"),
            voice.get("provider"),
            "edge"
        )),
    );
    entry.insert(
        "pitch".to_owned(),
        json!(coerce_voice_depth(definition.get("pitch"), 72.0)),
    );
    entry.insert(
        "locale".to_owned(),
        json!(py_or_text(voice.get("culture"), voice.get("locale"), "").trim()),
    );
    // How good the engine is, minted here so the picker sorts on one
    // authority rather than re-deriving the rule. Classified from the
    // catalog voice and not from this entry: an entry carries no
    // friendly_name and no description, so a tier that lived only in those
    // would be lost (`agent.py:2632-2636`).
    entry.insert("tier".to_owned(), json!(voice_tier(voice)));
    entry.insert("region".to_owned(), json!(country));
    entry.insert(
        "gender".to_owned(),
        json!(py_or_text(definition.get("sex"), voice.get("gender"), "")),
    );
    insert_person_fields(&mut entry, country, key, definition, &region);
    Some(Value::Object(entry))
}

/// The person half of a roster entry (`agent.py:2640-2649`): everything
/// above the split describes the voice, these are who the picker's info
/// icon shows.
fn insert_person_fields(
    entry: &mut Map<String, Value>,
    country: &str,
    key: &str,
    definition: &Value,
    region: &str,
) {
    entry.insert("source".to_owned(), json!("character"));
    // The picker's own identity for a character, minted here so the client
    // never re-derives it — and so nothing has to infer the kind. `region`
    // above is the country; this is the character's (`agent.py:2641-2645`).
    entry.insert("kind".to_owned(), json!("character"));
    entry.insert("character_region".to_owned(), json!(region));
    entry.insert(
        "dedupe_key".to_owned(),
        json!(character_dedupe_key(country, region, key)),
    );
    // Everything above describes the voice. These are the person, and they
    // are what the info icon beside the picker shows (`agent.py:2647-2649`).
    entry.insert("details".to_owned(), character_details(definition));
}

/// `str(voice.get("display_name") or voice_name)`: a falsy display name
/// reads as the voice's ShortName.
fn or_voice_name(value: Option<&Value>, voice_name: &str) -> String {
    match value {
        Some(value) => {
            let text = text_or_empty(Some(value));
            if text.is_empty() {
                voice_name.to_owned()
            } else {
                text
            }
        }
        None => voice_name.to_owned(),
    }
}

/// Python's `str(a or b or literal)`: the first truthy operand stringifies
/// and a falsy chain yields the literal. A truthy JSON value never stringifies
/// to empty — `text_or_empty` collapses exactly the falsy ones — so an empty
/// string here is exactly Python's falsy chain.
fn py_or_text(first: Option<&Value>, second: Option<&Value>, literal: &str) -> String {
    for value in [first, second].into_iter().flatten() {
        let text = text_or_empty(Some(value));
        if !text.is_empty() {
            return text;
        }
    }
    literal.to_owned()
}

/// `character_details` (`agent.py:2522`): the authored facts the picker's
/// info panel shows, or blanks for their absence.
///
/// Read from the country file's own entry rather than from a character
/// profile: the profile adapts these facts for a model — it fills in
/// defaults, sex-specific pronouns and register rules — and the panel has to
/// show what the file says (`agent.py:2525-2527`).
///
/// `region` is deliberately not among them. The picker entry already carries
/// it as `character_region`, which is the region its option label spends
/// itself on, and a second copy would be a second answer to "where is this
/// character from" (`agent.py:2529-2531`).
///
/// Every key is always present, blank when unauthored, so the client renders
/// one shape instead of guessing which of four fields a character happened to
/// write (`agent.py:2533-2535`).
#[must_use]
pub fn character_details(definition: &Value) -> Value {
    // `entry = definition if isinstance(definition, dict) else {}`
    let entry = definition.as_object().map_or(&Value::Null, |_| definition);
    let mut details = Map::new();
    details.insert(
        "born".to_owned(),
        json!(text_or_empty(entry.get("born")).trim()),
    );
    details.insert(
        "job".to_owned(),
        json!(text_or_empty(entry.get("job")).trim()),
    );
    details.insert(
        "backstory_lines".to_owned(),
        Value::Array(
            character_text_lines(entry.get("backstory_lines"))
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    details.insert(
        "backstory".to_owned(),
        json!(text_or_empty(entry.get("backstory")).trim()),
    );
    Value::Object(details)
}

/// `character_text_lines` (`agent.py:2545`): the lines of an authored list,
/// whether the file wrote a list or one string.
///
/// A single string is split on its newlines rather than iterated, because an
/// iterated string would be spelled out one character at a time
/// (`agent.py:2548-2549`).
#[must_use]
pub fn character_text_lines(value: Option<&Value>) -> Vec<String> {
    let candidates: Vec<String> = match value {
        Some(Value::Array(items)) => items.iter().map(py_str).collect(),
        Some(Value::String(text)) => py_splitlines(text).into_iter().map(str::to_owned).collect(),
        // `else: return []` — numbers, objects and null author no lines.
        _ => return Vec::new(),
    };
    candidates
        .iter()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// `character_dedupe_key` (`agent.py:2561`): the identity the picker dedupes
/// a character on: country, region and name.
///
/// Two characters are one entry only when all three agree, so characters
/// that happen to share a voice file never collapse into each other. Country
/// and region alone would not be enough: Canada ships Siqiniq in both
/// Inuktitut scripts under one region, and the script lives in `key`
/// (`agent.py:2564-2567`).
#[must_use]
pub fn character_dedupe_key(country: &str, region: &str, key: &str) -> String {
    format!(
        "character|{}|{}|{}",
        country.trim().to_lowercase(),
        region.trim().to_lowercase(),
        key.trim().to_lowercase()
    )
}
