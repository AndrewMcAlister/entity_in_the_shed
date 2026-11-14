//! The profile the prompt carries: whatever the model returned, flattened to one
//! short phrase per topic.
//!
//! Ported from `utils/conversation_history.py:312-399` — the four `USER_DETAILS_*`
//! constants (314-321), `_detail_value_text` (324), `_detail_label` (336) and
//! `normalize_user_details` (340). Pure: the model's reply arrives as the [`Value`]
//! the caller already parsed, a [`UserDetails`] leaves, and nothing in between
//! opens a file, reads a setting or asks the model for anything.
//!
//! ## The shape, and why it is flat
//!
//! One short line per topic, keyed by the topic itself — `name`, `family`,
//! `likes` — because a flat map is what keeps the rendered profile to a handful of
//! lines. `family` is just another topic: the source gives the person's people no
//! structure of their own. A list is folded into the same phrase form, so a model
//! answering `["cats", "jazz"]` and one answering `"cats, jazz"` store the same
//! thing, and the prompt's shape does not depend on which model answered.
//!
//! `other` is reserved: the catch-all for context with no natural topic of its
//! own. It is read before the topics and written after them, so
//! [`USER_DETAILS_MAX_FACTS`] cannot drop it, and it is allowed a paragraph
//! ([`USER_DETAILS_OTHER_MAX_CHARS`]) where a labelled topic gets a phrase
//! ([`USER_DETAILS_MAX_VALUE_CHARS`]).
//!
//! ## The `isinstance` ladder, as a JSON value
//!
//! | JSON | Phrase |
//! |---|---|
//! | string | whitespace collapsed to single spaces (`" ".join(value.split())`) |
//! | number | its JSON spelling, so `1.0` stays `"1.0"` where `f64::to_string` says `"1"` |
//! | bool | `""` — Python's `isinstance(True, int)` is true, so the source excludes bools by name |
//! | array | each element rendered by the same rule, empty parts dropped, joined `", "` |
//! | object, null | `""` |
//!
//! Truncation counts **characters**, because Python slices by code point: 200 of
//! them is 200 `é`s, not 200 bytes — the difference that only ever shows up in a
//! non-ASCII name.
//!
//! ## Two things the typed shape settles differently
//!
//! **A key is always text.** `str(key or "")` in the source has a `None` case
//! because a Python dict can be keyed by anything; a JSON key is text, so the
//! `{None: "no key"}` arm of the source's own test cannot be written here. The
//! `{"": value}` and `{label: ""}` arms of that test can, and are.
//!
//! **Topic order on the way in.** [`UserDetails`] keeps the order it is handed and
//! writes that order to the file, which is what the source's dict does — but what
//! it is handed here comes from a parsed `Value`, and `serde_json`'s object is a
//! `BTreeMap`, so the topics leave [`normalize_user_details`] in key order rather
//! than in the order the model listed them. Which facts survive the cap is
//! unaffected (the cap counts them, and `other` is kept apart from it either way),
//! and the prompt is one line per topic in both apps, so the only visible
//! difference is the order of those lines. A profile *loaded from a file* keeps
//! that file's order, which is the half the type exists for. Recorded in
//! `docs/PARITY.md`.

use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::py_strip;

/// How much of a person's profile the prompt can carry: one short line per topic.
/// The limits are the model's enthusiasm getting ahead of the prompt's budget.
pub const USER_DETAILS_MAX_FACTS: usize = 24;

/// The room a labelled topic has: a phrase.
pub const USER_DETAILS_MAX_VALUE_CHARS: usize = 200;

/// The reserved catch-all key, for context with no natural topic of its own.
pub const USER_DETAILS_OTHER_LABEL: &str = "other";

/// `other` is prose rather than a phrase, so its room is a paragraph's — and a
/// slot the topic cap above cannot take away.
pub const USER_DETAILS_OTHER_MAX_CHARS: usize = 512;

/// The profile of the person: a topic and its short phrase, in the order they were
/// written.
///
/// The order is the point, and it is why this type exists rather than a
/// `BTreeMap<String, String>`: Python's dict keeps the order the topics were
/// written in, and that order is what the file stores and what the rendered profile
/// shows. A map would silently sort them, so every rewrite of an old record would
/// reorder the person's own facts.
///
/// `Serialize` writes a JSON object in that order and `Deserialize` keeps the
/// document's. A document that names one topic twice keeps the **last** phrase at
/// the **first** position, which is what `json.loads` does and what
/// `details[label] = text` does, so the two apps store the same answer to the same
/// odd file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserDetails(Vec<(String, String)>);

impl UserDetails {
    /// A profile of these pairs, in this order.
    #[must_use]
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        Self(pairs)
    }

    /// The topics and their phrases, in stored order.
    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// The phrase stored under `label`.
    #[must_use]
    pub fn get(&self, label: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(stored, _)| stored == label)
            .map(|(_, text)| text.as_str())
    }

    /// How many topics are stored. `other` is one of them, and is never dropped to
    /// keep the count down — see [`normalize_user_details`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Store `text` under `label`, the way `details[label] = text` stores it over a
    /// dict that may already carry the key: the new phrase, at the position the key
    /// already had.
    pub fn set(&mut self, label: String, text: String) {
        match self.0.iter_mut().find(|(stored, _)| *stored == label) {
            Some((_, stored)) => *stored = text,
            None => self.0.push((label, text)),
        }
    }
}

/// Written as a JSON object, in stored order.
///
/// A derived impl could only produce an object by being a `BTreeMap`, which is the
/// sorting this type exists to avoid, so this writes the map itself:
/// `serialize_map` over the pairs in order is the object the source writes.
impl Serialize for UserDetails {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_map(Some(self.0.len()))?;
        for (label, text) in &self.0 {
            object.serialize_entry(label, text)?;
        }
        object.end()
    }
}

/// Read as a JSON object, keeping the document's order.
impl<'de> Deserialize<'de> for UserDetails {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UserDetailsVisitor)
    }
}

struct UserDetailsVisitor;

impl<'de> Visitor<'de> for UserDetailsVisitor {
    type Value = UserDetails;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object of topic to short phrase")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut details = UserDetails::default();
        while let Some((label, text)) = map.next_entry::<String, String>()? {
            details.set(label, text);
        }
        Ok(details)
    }
}

/// One profile value as a short phrase, or `""` when it is not one.
///
/// Ported from `_detail_value_text` (324-333), branch for branch. The branches that
/// read as `""` matter as much as the ones that read as text: a nested object is a
/// model answering with structure the flat profile has no room for, and a bool is
/// `""` rather than `"true"` because Python's `isinstance(True, int)` is true and
/// the source therefore names `bool` before its number branch.
fn detail_value_text(value: &Value) -> String {
    match value {
        Value::String(text) => py_join_whitespace(text),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(detail_value_text)
                .filter(|part| !part.is_empty())
                .collect();
            parts.join(", ")
        }
        Value::Bool(_) | Value::Null | Value::Object(_) => String::new(),
    }
}

/// A topic key in the stored form: lower case, spaces become underscores.
///
/// Ported from `_detail_label` (336-338), and in the source's own order: strip,
/// then lowercase, then replace — so a key padded with spaces cannot become
/// `_name_`. `str(key or "")` cannot fail here, because a JSON key is already text.
fn detail_label(key: &str) -> String {
    py_strip(key).to_lowercase().replace(' ', "_")
}

/// Python's `" ".join(value.split())`: every run of whitespace becomes one space,
/// and leading and trailing whitespace goes.
///
/// The same whitespace set [`py_strip`] trims with, restated because that helper is
/// a trim and this is a split: Python's `str.split()` cuts on `str.isspace()`, which
/// carries `\u{1c}`–`\u{1f}` that [`char::is_whitespace`] does not, and it drops the
/// empty parts a run of whitespace would otherwise produce. `str::split_whitespace`
/// is the same shape with the wrong set.
fn py_join_whitespace(value: &str) -> String {
    value
        .split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Python's `[:limit]`, which counts code points rather than bytes.
fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// A flat, prompt-sized profile from whatever the model returned.
///
/// Ported from `normalize_user_details` (340-372). Anything that is not an object
/// is no profile at all — the source's first line, and the answer to a model that
/// returned a string, a list or nothing.
///
/// `other` is read first and written last, so the [`USER_DETAILS_MAX_FACTS`] cap
/// cannot take its slot and its own [`USER_DETAILS_OTHER_MAX_CHARS`] room is not the
/// phrase budget the topics share. The source keeps the **first** `other` key it
/// meets and stops looking, which a model that hedged with both `"other"` and
/// `"Other"` gets in either app.
#[must_use]
pub fn normalize_user_details(value: &Value) -> UserDetails {
    let Value::Object(facts) = value else {
        return UserDetails::default();
    };

    let mut other = String::new();
    for (key, fact) in facts {
        if detail_label(key) == USER_DETAILS_OTHER_LABEL {
            other = take_chars(&detail_value_text(fact), USER_DETAILS_OTHER_MAX_CHARS);
            break;
        }
    }

    let mut details = UserDetails::default();
    for (key, fact) in facts {
        let label = detail_label(key);
        let text = detail_value_text(fact);
        if label.is_empty() || text.is_empty() || label == USER_DETAILS_OTHER_LABEL {
            continue;
        }
        details.set(label, take_chars(&text, USER_DETAILS_MAX_VALUE_CHARS));
        if details.len() >= USER_DETAILS_MAX_FACTS {
            break;
        }
    }

    if !other.is_empty() {
        details.set(USER_DETAILS_OTHER_LABEL.to_owned(), other);
    }
    details
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::{
        normalize_user_details, UserDetails, USER_DETAILS_MAX_FACTS, USER_DETAILS_MAX_VALUE_CHARS,
        USER_DETAILS_OTHER_MAX_CHARS,
    };

    /// `ProfileFactsTests` (`test_conversation_history_utils.py:230-308`).
    ///
    /// The Python assertions compare whole dicts, and a dict comparison ignores
    /// order. Whole-profile assertions here list the pairs in the order the clone
    /// produces them — key order, because a parsed `Value` is a map, see this
    /// module's doc — so the content is asserted exactly as the source asserts it.
    /// The order claim that matters, that a stored profile keeps the order its file
    /// wrote it in, is pinned by `tests/records.rs` and by
    /// [`a_profile_keeps_the_documents_order_and_a_duplicate_once`].
    fn facts(value: Value) -> UserDetails {
        normalize_user_details(&value)
    }

    /// A run of `count` `x`s, as Python's `"x" * count`.
    fn chars(count: usize) -> String {
        "x".repeat(count)
    }

    /// A profile as `(topic, phrase)` pairs, for whole-profile assertions.
    fn pairs(details: &UserDetails) -> Vec<(&str, &str)> {
        details
            .pairs()
            .iter()
            .map(|(label, text)| (label.as_str(), text.as_str()))
            .collect()
    }

    /// `{"fact 0": phrase, …}` for `count` topics, in the order the source's
    /// comprehension built them.
    fn topic_entries(count: usize, phrase: &str) -> Map<String, Value> {
        let entries: Vec<(String, Value)> = (0..count)
            .map(|index| (format!("fact {index}"), Value::String(phrase.to_owned())))
            .collect();
        entries.into_iter().collect()
    }

    /// A profile read from stored text; a panic here is a broken fixture.
    fn loaded(text: &str) -> UserDetails {
        match serde_json::from_str(text) {
            Ok(details) => details,
            Err(error) => panic!("{text} should load as a profile: {error}"),
        }
    }

    /// A profile as stored text; a panic here is a broken fixture.
    fn stored(details: &UserDetails) -> String {
        match serde_json::to_string(details) {
            Ok(text) => text,
            Err(error) => panic!("a profile should write: {error}"),
        }
    }

    #[test]
    fn labelled_facts_are_kept_as_short_phrases() {
        let details = facts(json!({
            "Name": "  Sam  ",
            "Town or City": "Bristol",
            "family": "their mum is unwell",
        }));

        assert_eq!(
            pairs(&details),
            [
                ("name", "Sam"),
                ("town_or_city", "Bristol"),
                ("family", "their mum is unwell"),
            ]
        );
    }

    #[test]
    fn a_list_of_preferences_becomes_one_phrase() {
        // A model answering with a list and one answering with a phrase must store
        // the same thing, or the prompt's shape depends on the model.
        let details = facts(json!({"likes": ["cats", "jazz", ""]}));

        assert_eq!(details.get("likes"), Some("cats, jazz"));
        // And the parts that are not phrases at all drop out of the join.
        assert_eq!(
            facts(json!({"likes": [Value::Null, "cats"]})).get("likes"),
            Some("cats")
        );
        assert!(facts(json!({"likes": []})).is_empty());
    }

    #[test]
    fn an_age_given_as_a_number_still_reads() {
        assert_eq!(facts(json!({"age": 34})).get("age"), Some("34"));
        // The JSON spelling rather than the float's own rendering, which would have
        // said `1` and lost what the model wrote.
        assert_eq!(facts(json!({"age": 1.0})).get("age"), Some("1.0"));
        // A bool is not a number, however much Python's `isinstance` wanted it to
        // be, so a model answering `true` stores nothing rather than "true".
        assert!(facts(json!({"alive": true})).is_empty());
    }

    #[test]
    fn whitespace_is_collapsed_to_one_line() {
        let details = facts(json!({"family": "a sister\n  and a sick\nmum"}));

        assert_eq!(details.get("family"), Some("a sister and a sick mum"));
    }

    #[test]
    fn empty_labels_and_values_are_dropped() {
        // The source's `{None: "no key"}` arm cannot be written here: a JSON key is
        // text, so a key that cannot exist is a case that cannot be tested.
        let details = facts(json!({
            "": "no label",
            "empty": "   ",
            "nested": {"too": "deep"},
        }));

        assert_eq!(details, UserDetails::default());
    }

    #[test]
    fn something_that_is_not_an_object_is_no_profile() {
        assert!(facts(Value::Null).is_empty());
        assert!(facts(json!("not a profile")).is_empty());
        assert!(facts(json!(["name", "Sam"])).is_empty());
    }

    #[test]
    fn the_profile_is_bounded() {
        let details = normalize_user_details(&Value::Object(topic_entries(
            USER_DETAILS_MAX_FACTS + 5,
            &chars(500),
        )));

        assert_eq!(details.len(), USER_DETAILS_MAX_FACTS);
        assert!(details
            .pairs()
            .iter()
            .all(|(_, text)| text.chars().count() <= USER_DETAILS_MAX_VALUE_CHARS));
    }

    #[test]
    fn other_is_a_catch_all_with_its_own_room() {
        // `other` holds what has no topic of its own, so it reads as prose and keeps
        // well past the phrase limit that fits a labelled fact.
        let details = facts(json!({"other": "  a long story\nabout a sick cat  "}));

        assert_eq!(details.get("other"), Some("a long story about a sick cat"));
    }

    #[test]
    fn other_is_cut_at_its_own_limit() {
        let details = facts(json!({"Other": chars(USER_DETAILS_OTHER_MAX_CHARS + 50)}));

        assert_eq!(
            details.get("other").map_or(0, |text| text.chars().count()),
            USER_DETAILS_OTHER_MAX_CHARS
        );
    }

    #[test]
    fn a_labelled_topic_does_not_get_others_room() {
        let details = facts(json!({"notes": chars(USER_DETAILS_OTHER_MAX_CHARS + 50)}));

        assert_eq!(
            details.get("notes").map_or(0, |text| text.chars().count()),
            USER_DETAILS_MAX_VALUE_CHARS
        );
    }

    #[test]
    fn other_is_kept_even_when_the_topic_count_is_full() {
        let mut entries = topic_entries(USER_DETAILS_MAX_FACTS, "x");
        entries.insert("other".to_owned(), json!("the boiler is broken"));

        let details = normalize_user_details(&Value::Object(entries));

        // The cap counts topics, and `other` is not one of the topics it counts.
        assert_eq!(details.len(), USER_DETAILS_MAX_FACTS + 1);
        assert_eq!(details.get("other"), Some("the boiler is broken"));
    }

    #[test]
    fn a_limit_counts_characters_not_bytes() {
        // Python slices by code point, so a phrase of accented text keeps 200 of
        // them. Reading the same limit as a byte budget would cut it short, and only
        // ever for a person whose name is not ASCII.
        let details = facts(json!({"name": "é".repeat(300)}));

        assert_eq!(
            details.get("name").map_or(0, |text| text.chars().count()),
            USER_DETAILS_MAX_VALUE_CHARS
        );
        assert_eq!(
            details.get("name").map_or(0, str::len),
            USER_DETAILS_MAX_VALUE_CHARS * 2
        );
    }

    #[test]
    fn two_spellings_of_one_topic_store_one_line() {
        // `name` and `Name` are the same topic once labelled, so the profile says it
        // once. Which of the two phrases survives follows the order the parse gave
        // (see this module's doc), so the phrase is not pinned here.
        let details = facts(json!({"name": "Sam", "Name": "Sam Spade"}));

        assert_eq!(pairs(&details).len(), 1);
        assert_eq!(details.pairs()[0].0, "name");
    }

    #[test]
    fn a_profile_keeps_the_documents_order_and_a_duplicate_once() {
        // The claim `UserDetails` exists for, tested on the stored bytes rather than
        // on a parsed `Value`: a loaded profile keeps the order its file wrote, and
        // a topic named twice keeps the last phrase at the first topic's place —
        // which is what `json.loads` does with a document like this.
        let details =
            loaded(r#"{"likes":"cats","name":"Sam","likes":"cats, jazz","family":"a sick mum"}"#);

        assert_eq!(
            pairs(&details),
            [
                ("likes", "cats, jazz"),
                ("name", "Sam"),
                ("family", "a sick mum"),
            ]
        );
        assert_eq!(
            stored(&details),
            r#"{"likes":"cats, jazz","name":"Sam","family":"a sick mum"}"#
        );
    }
}
