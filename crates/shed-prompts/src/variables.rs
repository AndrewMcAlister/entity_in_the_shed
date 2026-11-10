//! The variables interpolated into a mode template, and the rendering that
//! substitutes them.
//!
//! `_list_field_text`, `_prompt_template_variables` and
//! `render_prompt_variables` are `utils/prompt.py:36`, `:48` and `:103`.
//! The Python helper takes `character_facts` as a callable injected by the
//! `agent.py` wrapper; here the facts arrive precomputed, because the layer
//! map keeps this crate from reaching into `shed-characters` for them.

use std::collections::BTreeMap;

use fancy_regex::Regex;
use serde_json::Value;
use shed_text::{py_str, py_title, text_or_empty};

/// `str(profile.get(key, default))`: a present key is read through `str()`,
/// even when its value is null (which spells "None"); only a missing key
/// takes the default.
fn get_str(profile: &Value, key: &str, default: &str) -> String {
    match profile.get(key) {
        Some(value) => py_str(value),
        None => default.to_owned(),
    }
}

/// The `str(a or b or …)` idiom over profile and fact keys: the first value
/// that is not Python-falsy wins, untrimmed.
fn first_or_else(candidates: [Option<&Value>; 2], literal: &str) -> String {
    for candidate in candidates {
        let text = text_or_empty(candidate);
        if !text.is_empty() {
            return text;
        }
    }
    literal.to_owned()
}

/// The `.strip() or fallback` tail most of the variables carry.
fn trimmed_or(text: String, fallback: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// `_list_field_text` (`utils/prompt.py:36`): a list field joined into one
/// line with `"; "` — empty-after-trim items dropped — or a plain value read
/// as `str(value or "")` and trimmed.
///
/// Only a list is iterated, as in the source: a string author who wrote one
/// sentence instead of a list keeps the whole sentence.
#[must_use]
pub fn list_field_text(value: &Value) -> String {
    if let Value::Array(items) = value {
        return items
            .iter()
            .map(|item| py_str(item).trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect::<Vec<String>>()
            .join("; ");
    }
    text_or_empty(Some(value)).trim().to_owned()
}

/// The items `"\n".join(str(item).strip() for item in …)` iterates: a list's
/// elements, or — Python iterating a bare string — its characters. A number
/// or an object would raise `TypeError` there; no profile carries one and
/// neither contributes anything here.
fn joined_lines(value: Option<&Value>) -> String {
    let items: Vec<Value> = match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => text
            .chars()
            .map(|ch| Value::String(ch.to_string()))
            .collect(),
        _ => Vec::new(),
    };
    items
        .iter()
        .map(|item| py_str(item).trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<Vec<String>>()
        .join("\n")
}

/// `str(profile.get(key, "")).strip()` — a plain profile field.
fn plain_field(profile: &Value, key: &str) -> String {
    get_str(profile, key, "").trim().to_owned()
}

/// `str(profile.get(key, facts.get(key, ""))).strip()` — a profile field with
/// the persona fact behind it, taken only when the profile has no value for
/// the key at all (`utils/prompt.py:68-69/76-78`).
fn layered_field(profile: &Value, facts: &Value, key: &str) -> String {
    get_str(profile, key, &get_str(facts, key, ""))
        .trim()
        .to_owned()
}

/// `_list_field_text(profile.get(key, []))` — a list field joined onto one
/// line. A missing key and an authored null both read as the empty list the
/// source defaults to, so neither contributes text.
fn list_field(profile: &Value, key: &str) -> String {
    list_field_text(profile.get(key).unwrap_or(&Value::Null))
}

/// `str(profile.get("country") or profile.get("region") or …)`
/// (`utils/prompt.py:60`): a character's country, the region behind it, and
/// the English-speaking world behind both.
fn country_text(profile: &Value) -> String {
    const DEFAULT: &str = "the English-speaking world";
    trimmed_or(
        first_or_else([profile.get("country"), profile.get("region")], DEFAULT),
        DEFAULT,
    )
}

/// `str(profile.get("sex") or facts.get("sex") or "unknown").strip() or
/// "unknown"` (`utils/prompt.py:70`).
fn sex_text(profile: &Value, facts: &Value) -> String {
    const DEFAULT: &str = "unknown";
    trimmed_or(
        first_or_else([profile.get("sex"), facts.get("sex")], DEFAULT),
        DEFAULT,
    )
}

/// The profile-backed values, each with the two keys it is published under.
///
/// The source spells every one of these twice — once as the lower-case name a
/// template body uses (`utils/prompt.py:59-78`) and once as a title-case alias
/// with the underscores dropped (`:79-98`) — so each is computed once here and
/// inserted under both names by the caller.
fn profile_field_texts(
    profile: &Value,
    facts: &Value,
) -> Vec<(&'static str, &'static str, String)> {
    vec![
        (
            "name",
            "Name",
            trimmed_or(get_str(profile, "name", "Greg"), "Greg"),
        ),
        ("country", "Country", country_text(profile)),
        ("born", "Born", plain_field(profile, "born")),
        (
            "mannerisms",
            "Mannerisms",
            list_field(profile, "mannerisms"),
        ),
        (
            "speech_register",
            "SpeechRegister",
            list_field(profile, "speech_register"),
        ),
        ("backstory", "Backstory", plain_field(profile, "backstory")),
        (
            "backstory_lines",
            "BackstoryLines",
            joined_lines(profile.get("backstory_lines")),
        ),
        ("locale", "Locale", plain_field(profile, "locale")),
        ("region", "Region", plain_field(profile, "region")),
        (
            "home_region",
            "HomeRegion",
            layered_field(profile, facts, "region"),
        ),
        (
            "origin_detail",
            "OriginDetail",
            layered_field(profile, facts, "origin_detail"),
        ),
        (
            "voice_style",
            "VoiceStyle",
            layered_field(profile, facts, "voice_style"),
        ),
        (
            "political_temperament",
            "PoliticalTemperament",
            layered_field(profile, facts, "political_temperament"),
        ),
        ("job", "Job", layered_field(profile, facts, "job")),
    ]
}

/// The five pronouns, read once from the persona facts and published twice
/// (`utils/prompt.py:71-75/91-95`): under the lower-case key the trimmed fact,
/// under the title-case key `str.title()` of the same text, each falling back
/// to its own default — which is why the two columns of defaults differ.
fn pronoun_texts(facts: &Value) -> Vec<(String, String)> {
    const PRONOUNS: [(&str, &str, &str, &str); 5] = [
        ("subject_pronoun", "SubjectPronoun", "they", "They"),
        ("object_pronoun", "ObjectPronoun", "them", "Them"),
        (
            "possessive_adjective",
            "PossessiveAdjective",
            "their",
            "Their",
        ),
        (
            "possessive_pronoun",
            "PossessivePronoun",
            "theirs",
            "Theirs",
        ),
        (
            "reflexive_pronoun",
            "ReflexivePronoun",
            "themself",
            "Themself",
        ),
    ];
    let mut texts = Vec::new();
    for (key, titled_key, plain_default, titled_default) in PRONOUNS {
        let raw = get_str(facts, key, plain_default).trim().to_owned();
        texts.push(((*key).to_owned(), trimmed_or(raw.clone(), plain_default)));
        texts.push((
            (*titled_key).to_owned(),
            trimmed_or(py_title(&raw), titled_default),
        ));
    }
    texts
}

/// `_prompt_template_variables` (`utils/prompt.py:48`): every name a mode
/// template can interpolate, from the character profile and the persona
/// facts derived from it.
///
/// `facts` is what the Python wrapper's `character_facts(profile)` would
/// return; `None` profile reads as the empty dict it collapses to.
#[must_use]
pub fn prompt_template_variables(
    character_profile: Option<&Value>,
    user_text: &Value,
    facts: &Value,
) -> BTreeMap<String, String> {
    let profile = character_profile.unwrap_or(&Value::Null);
    let mut variables = BTreeMap::new();

    for (key, titled_key, value) in profile_field_texts(profile, facts) {
        variables.insert((*key).to_owned(), value.clone());
        variables.insert((*titled_key).to_owned(), value);
    }
    for (key, value) in pronoun_texts(facts) {
        variables.insert(key, value);
    }

    // `sex` is the resolved sex and `Sex` the same word titled
    // (`utils/prompt.py:70/90`) — the one pair whose two keys do not carry the
    // same text. `input` has no alias: it is the user's own words.
    let sex = sex_text(profile, facts);
    variables.insert("sex".to_owned(), sex.clone());
    variables.insert("Sex".to_owned(), py_title(&sex));
    variables.insert(
        "input".to_owned(),
        text_or_empty(Some(user_text)).trim().to_owned(),
    );

    variables
}

/// `render_prompt_variables` (`utils/prompt.py:103`): `{{name}}` and `{name}`
/// both take their variable, unknown names go empty, three or more newlines
/// collapse to two, and the result is trimmed.
///
/// `{{cache:…}}` is not a variable — the colon is outside the name class —
/// so a prompt-cache placeholder survives rendering untouched.
#[must_use]
pub fn render_prompt_variables(
    prompt_text: &Value,
    variables: &BTreeMap<String, String>,
) -> String {
    static VARIABLES: std::sync::OnceLock<Option<Regex>> = std::sync::OnceLock::new();
    static NEWLINES: std::sync::OnceLock<Option<Regex>> = std::sync::OnceLock::new();
    let variables_re = VARIABLES
        .get_or_init(|| Regex::new(r"\{\{\s*([A-Za-z0-9_]+)\s*\}\}|\{\s*([A-Za-z0-9_]+)\s*\}").ok())
        .as_ref();
    let newlines_re = NEWLINES.get_or_init(|| Regex::new(r"\n{3,}").ok()).as_ref();

    let text = text_or_empty(Some(prompt_text));
    let rendered = match variables_re {
        Some(re) => re
            .replace_all(&text, |caps: &fancy_regex::Captures<'_, str>| {
                let key = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map(|name| name.as_str())
                    .unwrap_or("")
                    .trim();
                variables.get(key).cloned().unwrap_or_default()
            })
            .into_owned(),
        // A pattern that fails to compile reads as absent; neither does.
        None => text,
    };
    let collapsed = match newlines_re {
        Some(re) => re.replace_all(&rendered, "\n\n").into_owned(),
        None => rendered,
    };
    collapsed.trim().to_owned()
}
