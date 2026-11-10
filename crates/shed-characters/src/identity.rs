//! Character identity: the key a country file's entry gets, the id the picker
//! hands out, and the cache tag a character's prompt is stored under.
//!
//! Ported from `utils/characters.py` unchanged in behaviour. Identity is
//! deliberately derived from nothing but the country name and the entry's own
//! authored name and voice, so the same character gets the same id and the same
//! conversation folder on every machine and every restart.

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use shed_text::text_or_empty;

/// Character ids are `"<Country>:<key>"`, and the key also names the
/// character's conversation folder as `"<country>-<key>"`.
pub const CHARACTER_ID_SEPARATOR: char = ':';

/// The prompt-cache namespace a character identity is digested under
/// (`utils/characters.py:23`).
pub const CHARACTER_CACHE_NAMESPACE: &str = "entity-character:";

/// How many hex characters of that digest name the character (`utils/characters.py:24`).
pub const CHARACTER_CACHE_ID_LENGTH: usize = 16;

/// A short, stable tag distinguishing same-named characters
/// (`utils/characters.py:27`).
///
/// Prefers the voice's script subtag (`"iu-Cans-CA"` → `"cans"`), which is what
/// usually differs, and falls back to the ordinal.
#[must_use]
pub fn character_key_suffix(entry: &Value, ordinal: i64) -> String {
    let voice = text_or_empty(entry.get("voice"));
    let parts: Vec<&str> = voice.split('-').collect();
    // Python's `parts[1:-1]`: the language and country subtags are never the
    // script, and a name with fewer than three parts has no middle to scan.
    if parts.len() >= 3 {
        for part in &parts[1..parts.len() - 1] {
            if part.chars().count() == 4 && part.chars().all(char::is_alphabetic) {
                return part.to_lowercase();
            }
        }
    }
    ordinal.to_string()
}

/// `{key: entry}` for the parsed `characters` list of one country file
/// (`utils/characters.py:39`), kept as an ordered vec of pairs.
///
/// The order is the file's declaration order, which is the order the picker
/// shows, so a `HashMap` could not stand in for Python's insertion-ordered
/// dict here.
///
/// The key is the character's name, disambiguated only when a country declares
/// the same name twice. The first occurrence keeps the bare name, so the
/// primary character's conversation folder stays clean.
#[must_use]
pub fn character_keys_from_entries(characters: &[Value]) -> Vec<(String, Value)> {
    let entries: Vec<&Value> = characters
        .iter()
        .filter(|entry| entry.is_object())
        .collect();

    let mut counts = Tally::default();
    for entry in &entries {
        let name = text_or_empty(entry.get("name")).trim().to_owned();
        if !name.is_empty() {
            counts.bump(&name);
        }
    }

    let mut keys: Vec<(String, Value)> = Vec::new();
    let mut seen = Tally::default();
    for entry in &entries {
        let name = text_or_empty(entry.get("name")).trim().to_owned();
        if name.is_empty() {
            continue;
        }

        let seen_count = seen.bump(&name);
        let declared = counts.count(&name);

        let mut key = if declared < 2 || seen_count == 1 {
            name.clone()
        } else {
            tagged_key(&name, entry, seen_count)
        };

        while keys.iter().any(|(taken, _)| *taken == key) {
            key = tagged_key(&name, entry, seen.bump(&name));
        }

        keys.push((key, (*entry).clone()));
    }

    keys
}

/// Python's `dict.get(name, 0)` with a `+= 1` write-back, in declaration
/// order. Country files hold a handful of names, so the linear scan is the
/// point — a hash map would only lose the order `keys` must keep anyway.
#[derive(Default)]
struct Tally(Vec<(String, usize)>);

impl Tally {
    /// `counts[name] = counts.get(name, 0) + 1`, returning the new count.
    fn bump(&mut self, name: &str) -> usize {
        match self.0.iter_mut().find(|(known, _)| known == name) {
            Some((_, count)) => {
                *count += 1;
                *count
            }
            None => {
                self.0.push((name.to_owned(), 1));
                1
            }
        }
    }

    /// `counts.get(name, 0)`.
    fn count(&self, name: &str) -> usize {
        self.0
            .iter()
            .find(|(known, _)| known == name)
            .map_or(0, |(_, count)| *count)
    }
}

/// `"{name}-{suffix}"` — the disambiguated form of a repeated name.
fn tagged_key(name: &str, entry: &Value, ordinal: usize) -> String {
    let ordinal = i64::try_from(ordinal).unwrap_or(i64::MAX);
    format!("{name}-{}", character_key_suffix(entry, ordinal))
}

/// The picker id for a character: `"<Country>:<key>"` (`utils/characters.py:76`).
#[must_use]
pub fn character_id(country: &str, key: &str) -> String {
    format!("{country}{CHARACTER_ID_SEPARATOR}{key}")
}

/// `(country, key)` for an id this app minted, else `("", "")`
/// (`utils/characters.py:81`).
///
/// Only the first separator splits: a key never contains one, but a stray id
/// from elsewhere might, and Python's `partition` keeps the remainder whole.
#[must_use]
pub fn split_character_id(character_id: &str) -> (String, String) {
    let raw = character_id.trim();
    if raw.is_empty() {
        return (String::new(), String::new());
    }
    match raw.split_once(CHARACTER_ID_SEPARATOR) {
        Some((country, key)) => (country.trim().to_owned(), key.trim().to_owned()),
        None => (String::new(), String::new()),
    }
}

/// The prompt-cache tag for a character identity, e.g. a hash of `"Canada:Siqiniq"`
/// (`utils/characters.py:90`).
///
/// Deterministic and stable across restarts and machines, which is what makes
/// one character's prompt a single named block reusable by every user who
/// picks it. Sixteen hex characters name 64 bits: two collisions among the few
/// hundred characters shipped are not a risk worth spending a name on.
#[must_use]
pub fn character_cache_id(identifier: &str) -> String {
    let text = identifier.trim();
    if text.is_empty() {
        return String::new();
    }
    let digest = Sha256::digest(format!("{CHARACTER_CACHE_NAMESPACE}{text}").as_bytes());
    let hex = format!("{digest:x}");
    hex[..CHARACTER_CACHE_ID_LENGTH].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_script_subtag_is_the_suffix() {
        let entry = json!({"name": "Siqiniq", "voice": "iu-Cans-CA-SiqiniqNeural"});
        assert_eq!(character_key_suffix(&entry, 2), "cans");
        let latin = json!({"name": "Siqiniq", "voice": "iu-Latn-CA-SiqiniqNeural"});
        assert_eq!(character_key_suffix(&latin, 2), "latn");
    }

    #[test]
    fn a_voice_without_a_middle_subtag_falls_back_to_the_ordinal() {
        let entry = json!({"name": "Ann", "voice": "en-GB-AnneNeural"});
        assert_eq!(character_key_suffix(&entry, 2), "2");
        // The language and country subtags are skipped, never scanned.
        let short = json!({"voice": "abcd-efgh"});
        assert_eq!(character_key_suffix(&short, 3), "3");
    }

    #[test]
    fn a_lone_name_keeps_its_bare_key() {
        let keys = character_keys_from_entries(&[json!({"name": "Ada", "voice": "en-GB-A"})]);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0, "Ada");
    }

    #[test]
    fn the_first_of_two_keeps_the_bare_name_and_the_second_is_tagged() {
        let keys = character_keys_from_entries(&[
            json!({"name": "Siqiniq", "voice": "iu-Cans-CA-SiqiniqNeural"}),
            json!({"name": "Siqiniq", "voice": "iu-Latn-CA-SiqiniqNeural"}),
        ]);
        let names: Vec<&str> = keys.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(names, ["Siqiniq", "Siqiniq-latn"]);
    }

    #[test]
    fn a_suffix_collision_keeps_bumping_the_ordinal() {
        // Three same-named characters whose voices carry no script subtag: the
        // second gets "-2", and the third must not collide into "-2" again.
        let keys = character_keys_from_entries(&[
            json!({"name": "Ann", "voice": "en-GB-A"}),
            json!({"name": "Ann", "voice": "en-GB-A"}),
            json!({"name": "Ann", "voice": "en-GB-A"}),
        ]);
        let names: Vec<&str> = keys.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(names, ["Ann", "Ann-2", "Ann-3"]);
    }

    #[test]
    fn blank_and_non_object_entries_are_dropped() {
        let keys = character_keys_from_entries(&[
            json!("not an object"),
            json!({"voice": "en-GB-A"}),
            json!({"name": "   ", "voice": "en-GB-B"}),
            json!({"name": "Ada", "voice": "en-GB-C"}),
        ]);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0, "Ada");
    }

    #[test]
    fn ids_join_and_split_back_to_their_parts() {
        assert_eq!(
            character_id("United Kingdom", "Beta"),
            "United Kingdom:Beta"
        );
        assert_eq!(
            split_character_id("United Kingdom:Beta"),
            ("United Kingdom".to_owned(), "Beta".to_owned())
        );
    }

    #[test]
    fn an_id_we_did_not_mint_has_no_parts() {
        for not_ours in ["en-GB-RyanNeural", "", "nonsense"] {
            assert_eq!(split_character_id(not_ours), (String::new(), String::new()));
        }
    }

    #[test]
    fn the_cache_id_is_a_stable_sixteen_hex_digest() {
        let first = character_cache_id("Canada:Siqiniq");
        assert_eq!(first.len(), CHARACTER_CACHE_ID_LENGTH);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(first, character_cache_id("Canada:Siqiniq"));
        assert_ne!(first, character_cache_id("Canada:Siqiniq-latn"));
        assert_eq!(character_cache_id("  Canada:Siqiniq  "), first);
        assert_eq!(character_cache_id(""), "");
    }

    #[test]
    fn the_digest_is_the_one_python_mints() {
        // The namespace and the digest length are shared with the Python
        // side by construction — `utils/characters.py:23` — but the value
        // itself can only be checked against an outside record. These two
        // are what `hashlib.sha256` produced for these identities, and what
        // `shed-prompts`' port of `test_prompt_cache.py` quotes back.
        assert_eq!(character_cache_id("Canada:Siqiniq"), "e8ca9d6ec25664b4");
        assert_eq!(character_cache_id("Testland:Aria"), "0be1f85097f5414b");
    }
}
