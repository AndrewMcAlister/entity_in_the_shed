//! The persona: the default template, the profile built from a voice pick,
//! and the system prompt that persona is written into.
//!
//! Ported from `agent.py:2735` (`format_character_template_text`),
//! `agent.py:2746` (`normalize_character_list`), `agent.py:2761`
//! (`build_backstory_lines`), `agent.py:2777` (`normalize_location_phrase`),
//! `agent.py:2783` (`build_character_fact_sheet`), `agent.py:2803`
//! (`build_character_profile`), `agent.py:2992` (`build_character_system_prompt`),
//! `agent.py:3041` (`public_character_profile`), `agent.py:2277`
//! (`template_value`), `agent.py:2284` (`default_character_templates`), and
//! `agent.py:4690` (`character_facts`).
//!
//! [`CharacterCatalog::build_character_profile`] is a method rather than a
//! free function because resolving a character means reading its country file
//! — `character_definition`, `character_folder` and `character_baseline_pitch`
//! all come from the catalog's caches.
//!
//! Two pieces of `agent.py`'s persona area are deliberately not here:
//! `ensure_character_outline` (`agent.py:2964`) writes into `MEMORY_DIR` and
//! belongs with the conversation folders, and `character_prompt_cache_id`
//! (`agent.py:3062`) needs `is_prompt_block_name` from the prompt cache.
//! Neither has a caller until those stages land.

use std::sync::OnceLock;

use serde_json::{json, Map, Value};
use shed_text::{normalize_locale, py_repr, py_splitlines, py_str, py_title, text_or_empty};

use crate::catalog::CharacterCatalog;
use crate::identity::split_character_id;
use crate::picker::slugify_character_name;
use crate::sex::{
    adapt_character_list_for_sex, adapt_character_text_for_sex, infer_character_sex,
    normalize_character_sex, pronouns_for_character_sex, Pronouns, Sex,
};

/// Python truthiness over a JSON value — the `bool(x)` an `or` chain asks.
/// A NaN number is truthy exactly as `bool(float("nan"))` is, and `-0.0`
/// falsy exactly as Python's is.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_none_or(|value| value != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
    }
}

/// The text of `str(a or b or ... or "")`: the first truthy candidate,
/// stringified the way Python's `str()` would, or `""` when none is.
fn or_text(candidates: &[Option<&Value>]) -> String {
    candidates
        .iter()
        .copied()
        .flatten()
        .find(|value| truthy(Some(value)))
        .map_or_else(String::new, py_str)
}

/// `str(container.get(key, default))`: a present key returns its value even
/// when that value is `null` — `str(None)` spells `"None"` downstream — and
/// only an absent key takes `default`. The persona and prompt builders mix
/// these with `or` chains, and the two read an authored null differently.
fn get_text(container: &Value, key: &str, default: &str) -> String {
    match container.get(key) {
        Some(found) => py_str(found),
        None => default.to_owned(),
    }
}

/// `template_value` (`agent.py:2277`): the first key present with a value
/// that is neither `None` nor `""`, else `default`.
///
/// The source's filter is `not in (None, "")`, which is equality against
/// those two values only — a authored `0` or `false` passes through and the
/// caller's `str()` then spells it "0"/"False".
fn template_value<'a>(template: &'a Value, keys: &[&str], default: &'a Value) -> &'a Value {
    keys.iter()
        .find_map(|key| {
            let found = template.get(*key)?;
            let skipped = match found {
                Value::Null => true,
                Value::String(text) => text.is_empty(),
                _ => false,
            };
            (!skipped).then_some(found)
        })
        .unwrap_or(default)
}

/// `format_character_template_text` (`agent.py:2735`): `str(value or "").strip()`,
/// then Python's `str.format` over the persona's `format_values`.
///
/// Python's format raises on an unknown field, a positional field, an
/// attribute part or a malformed brace, and the source catches every
/// exception and returns the unformatted text. [`py_format`] folds all of
/// those into `None`, so the error path is the same text. A field carrying a
/// format spec (`{name:>10}`) is the one case where Python *succeeds* while
/// this returns the raw text; no shipped country file and not the default
/// persona use one — see the module note in `docs/PARITY.md`.
fn format_character_template_text(value: &Value, values: &Map<String, Value>) -> String {
    let text = text_or_empty(Some(value)).trim().to_owned();
    if text.is_empty() {
        return String::new();
    }
    match py_format(&text, values) {
        Some(formatted) => formatted.trim().to_owned(),
        None => text,
    }
}

/// Python's `str.format(**values)` for the persona's all-string kwargs.
/// `None` is any exception the call would raise — the caller then keeps the
/// raw text, exactly as the source's bare `except Exception` does.
fn py_format(text: &str, values: &Map<String, Value>) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '{' if chars.get(index + 1) == Some(&'{') => {
                out.push('{');
                index += 2;
            }
            '{' => {
                let (field, next) = scan_field(&chars, index + 1)?;
                out.push_str(&substitute(field, values)?);
                index = next;
            }
            '}' if chars.get(index + 1) == Some(&'}') => {
                out.push('}');
                index += 2;
            }
            '}' => return None, // "Single '}' encountered in format string"
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    Some(out)
}

/// The text between a `{` (already consumed, at `start`) and its matching
/// `}`, plus the index just past that brace. Nested braces belong to a
/// format spec; an unclosed field is a `ValueError`.
fn scan_field(chars: &[char], start: usize) -> Option<(&[char], usize)> {
    let mut depth = 0_usize;
    let mut end = start;
    while end < chars.len() {
        match chars[end] {
            '{' => depth += 1,
            '}' if depth == 0 => return Some((&chars[start..end], end + 1)),
            '}' => depth -= 1,
            _ => {}
        }
        end += 1;
    }
    None // "expected '}' before end of string"
}

/// Resolve one field: `{name}`, `{name!s}`, `{name!r}` and `{name:}` succeed
/// when `name` is a key; anything else is an exception path. An unknown name
/// is a `KeyError`, a positional or empty field an `IndexError`/`ValueError`,
/// and an attribute part an `AttributeError` on the string value. A non-empty
/// format spec would succeed in Python but is treated as an error here — the
/// one documented divergence above.
fn substitute(field: &[char], values: &Map<String, Value>) -> Option<String> {
    let text: String = field.iter().collect();
    let boundary = text.find(['!', ':']).unwrap_or(text.len());
    let name = &text[..boundary];
    let rest = &text[boundary..];

    if name.is_empty() || name.contains(['.', '[']) {
        return None;
    }
    let value = py_str(values.get(name)?);

    if rest.is_empty() || rest == ":" {
        return Some(value);
    }
    if let Some(conversion) = rest.strip_prefix('!') {
        let mut marks = conversion.chars();
        let mark = marks.next()?;
        let tail = marks.as_str();
        if !(tail.is_empty() || tail == ":") {
            return None; // a spec after the conversion: the documented path
        }
        return Some(match mark {
            's' => value,
            'r' | 'a' => py_repr(&Value::String(value)),
            _ => return None,
        });
    }
    None // a non-empty format spec
}

/// `template_value`'s `default=[]` for the persona's list fields.
static EMPTY_LIST: Value = Value::Array(Vec::new());

/// `template_value`'s `default=""` for the persona's text fields.
static EMPTY_TEXT: Value = Value::String(String::new());

/// The persona field `str(template_value(template, *keys, default)).strip()`
/// takes. Only the list fields go elsewhere.
fn template_text(template: &Value, keys: &[&str], default: &Value) -> String {
    py_str(template_value(template, keys, default))
        .trim()
        .to_owned()
}

/// `text or fallback` once `text` has already been stripped — the shape of
/// `str(...).strip() or "Greg"`.
fn or_fallback(text: String, fallback: &str) -> String {
    if text.is_empty() {
        fallback.to_owned()
    } else {
        text
    }
}

/// `normalize_character_list` (`agent.py:2746`): a list formats item by item,
/// a string line by line, anything else contributes nothing; entries that
/// format to empty are dropped.
///
/// The string branch splits on Python's line breaks — an authored
/// `"first\nsecond"` is two mannerisms — which is what separates this from
/// [`build_backstory_lines`], whose source iterates a bare string one
/// character at a time.
fn normalize_character_list(value: &Value, values: &Map<String, Value>) -> Vec<Value> {
    let candidates: Vec<Value> = match value {
        Value::Array(items) => items.clone(),
        Value::String(text) => py_splitlines(text)
            .into_iter()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| Value::String(line.to_owned()))
            .collect(),
        _ => Vec::new(),
    };
    formatted_lines(&Value::Array(candidates), values)
}

/// The items a persona list field iterates (`agent.py:2764`): a list's
/// elements, or — Python iterating a bare string — its characters. A number
/// or an object would raise `TypeError` there; no country file authors one
/// and neither contributes anything here.
fn iterable_lines(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::String(text) => text
            .chars()
            .map(|ch| Value::String(ch.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// The `format_character_template_text(line, ...) for line in ... if ...`
/// comprehension both persona list builders are written with.
fn formatted_lines(source: &Value, values: &Map<String, Value>) -> Vec<Value> {
    iterable_lines(source)
        .iter()
        .map(|line| Value::String(format_character_template_text(line, values)))
        .filter(|line| truthy(Some(line)))
        .collect()
}

/// `build_backstory_lines` (`agent.py:2761`): the character's authored lines,
/// and the default persona's only when it authors none.
///
/// The fallback is the *default template's* list, which the caller passes in
/// already resolved — the source hands it the same `template_value` result it
/// would have taken from `default_character_templates()["default"]`.
fn build_backstory_lines(
    template: &Value,
    values: &Map<String, Value>,
    fallback_lines: &Value,
) -> Vec<Value> {
    let authored = formatted_lines(
        template_value(
            template,
            &["backstory_lines", "BackstoryLines", "Backstory_lines"],
            &EMPTY_LIST,
        ),
        values,
    );
    if authored.is_empty() {
        return formatted_lines(fallback_lines, values);
    }
    authored
}

/// `normalize_location_phrase` (`agent.py:2777`): trimmed, with the trailing
/// punctuation Python's `rstrip(" .,!?:;")` set peeled off, then whitespace
/// collapsed to single spaces.
///
/// The `strip()` runs before the `rstrip`, so `"London. "` loses both the
/// period and the space, while `"St. James"` keeps its interior period —
/// `rstrip` only ever works from the right.
fn normalize_location_phrase(value: &Value) -> String {
    let text = or_text(&[Some(value)]);
    let trimmed = text
        .trim()
        .trim_end_matches([' ', '.', ',', '!', '?', ':', ';']);
    collapsed_spaces(trimmed)
}

/// `re.sub(r"\s+", " ", text)` — Python's `\s` over the strings persona text
/// actually holds: the ASCII runs plus the Unicode spaces `str.isspace` sees.
fn collapsed_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(ch);
    }
    out
}

/// `normalize_location_phrase(a or b or ...)` — the chain the fact sheet
/// resolves its places with before the phrase is cleaned.
fn phrase(candidates: &[Option<&Value>]) -> String {
    normalize_location_phrase(&Value::String(or_text(candidates)))
}

/// The eight authored fields a fact sheet is written from — the parameters
/// `build_character_fact_sheet` takes.
///
/// Owned `Value`s rather than borrows because the two callers resolve them
/// from different places: the persona builder has its own resolved strings,
/// [`character_facts`] reads them back out of a saved profile.
struct FactFields {
    name: Value,
    region: Value,
    country: Value,
    origin_detail: Value,
    sex: Value,
    voice_style: Value,
    political_temperament: Value,
    job: Value,
}

impl FactFields {
    /// The shape [`character_facts`] reads: the same eight keys a profile
    /// writes, which is also the shape `build_character_fact_sheet`'s
    /// `profile.get(key, "")` arguments take — hence the empty-string default.
    fn from_profile(profile: &Value) -> Self {
        let field = |key: &str| {
            profile
                .get(key)
                .cloned()
                .unwrap_or_else(|| EMPTY_TEXT.clone())
        };
        FactFields {
            name: field("name"),
            region: field("region"),
            country: field("country"),
            origin_detail: field("origin_detail"),
            sex: field("sex"),
            voice_style: field("voice_style"),
            political_temperament: field("political_temperament"),
            job: field("job"),
        }
    }
}

/// `build_character_fact_sheet` (`agent.py:2783`): the eight authored fields
/// the self-questioning answers from, plus the pronoun set `sex` selects.
///
/// Every field is `str(x or "").strip()`, so an authored `null` reads as the
/// empty string here even though the persona builder's `template.get` would
/// have spelled it "None" — the two paths differ in the source and in Rust.
/// `region` falls through to the country and `origin_detail` through region to
/// the country, which is how a character with no subdivision still gets a
/// place to say it is from.
fn build_character_fact_sheet(fields: &FactFields) -> Value {
    let FactFields {
        name,
        region,
        country,
        origin_detail,
        sex,
        voice_style,
        political_temperament,
        job,
    } = fields;
    let stripped = |value: &Value| or_text(&[Some(value)]).trim().to_owned();
    let origin_summary = or_text(&[Some(country), Some(region)]);

    let mut facts = Map::new();
    for (key, value) in [
        ("identity_name", stripped(name)),
        ("origin_summary", origin_summary.trim().to_owned()),
        ("region", phrase(&[Some(region), Some(country)])),
        (
            "origin_detail",
            phrase(&[Some(origin_detail), Some(region), Some(country)]),
        ),
        ("voice_style", stripped(voice_style)),
        ("political_temperament", stripped(political_temperament)),
        ("job", stripped(job)),
    ] {
        facts.insert(key.to_owned(), Value::String(value));
    }
    // fact_sheet.update(pronouns_for_character_sex(sex))
    facts.extend(pronouns_for_character_sex(normalize_character_sex(sex)));
    Value::Object(facts)
}

/// `default_character_templates` (`agent.py:2284`): the code's own persona,
/// "Greg". The only template the dict holds; a country file that names no
/// character, and a voice picked without a country, both fall back on it.
///
/// Python rebuilds the dict on every call. This builds it once and hands out
/// the same read-only copy — nothing in the persona builder writes to the
/// template it was given, and a caller that needs to write takes a `clone()`.
#[must_use]
pub fn default_character_templates() -> &'static Map<String, Value> {
    static TEMPLATES: OnceLock<Map<String, Value>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let mut greg = Map::new();
        for (key, value) in [
            ("region", json!("the English-speaking world")),
            ("name", json!("Greg")),
            ("sex", json!("male")),
            ("country", json!("the English-speaking world")),
            ("born", json!("1948")),
            ("job", json!("Itinerant raconteur")),
            ("backstory", json!(GREG_BACKSTORY)),
            (
                "mannerisms",
                json!([
                    "Lets a thought breathe for a beat before answering, as if he is weighing it by lamplight.",
                    "Favours dry asides, old-fashioned turns of phrase, and the occasional pointed eyebrow in verbal form.",
                    "Treats even ordinary news as something worth framing properly before he gives his verdict.",
                ]),
            ),
            (
                "backstory_lines",
                json!([
                    "{name} is a wandering raconteur from {region}, forever turning ordinary events into a tale worth retelling.",
                    "He has a habit of sounding as though he has stepped out of a smoky old stage play, even when speaking about modern affairs.",
                    "He prizes useful answers, but cannot resist adding a morsel of personality, judgment, and scene-setting to the telling.",
                ]),
            ),
        ] {
            greg.insert(key.to_owned(), value);
        }

        let mut templates = Map::new();
        templates.insert("default".to_owned(), Value::Object(greg));
        templates
    })
}

/// The `"default"` entry of [`default_character_templates`], which is how
/// every caller in the source reaches it.
fn default_template() -> &'static Value {
    default_character_templates()
        .get("default")
        .unwrap_or(&Value::Null)
}

/// The default persona's `backstory_lines`, which is the fallback
/// [`build_backstory_lines`] is called with.
fn default_backstory_lines() -> &'static Value {
    template_value(
        default_template(),
        &["backstory_lines", "BackstoryLines", "Backstory_lines"],
        &EMPTY_LIST,
    )
}

/// `a or b or ... or fallback` over strings that have already been through
/// Python's `str(...).strip()`, which is the only thing `or` inspects there.
fn first_non_empty<'a>(parts: &[&'a str], fallback: &'a str) -> String {
    parts
        .iter()
        .copied()
        .find(|part| !part.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// `voice_sex or template_sex or inferred_name_sex or "unknown"`
/// (`agent.py:2840`): the first that says anything wins, and [`Sex::Unstated`]
/// is the falsy one — which is precisely why the enum carries it apart from
/// [`Sex::Unknown`], a stated sex that is not male or female.
fn decided_sex(candidates: &[Sex]) -> Sex {
    candidates
        .iter()
        .copied()
        .find(|sex| !matches!(sex, Sex::Unstated))
        .unwrap_or(Sex::Unknown)
}

/// `voice_sex or template_sex or inferred_name_sex or "unknown"`
/// (`agent.py:2837-2840`): the gender the voice states wins over the sex the
/// character authors, which wins over what the name table reads.
fn persona_sex(voice: &Map<String, Value>, template: &Value, name: &str, voice_label: &str) -> Sex {
    decided_sex(&[
        normalize_character_sex(&Value::String(or_text(&[
            voice.get("gender"),
            voice.get("Gender"),
        ]))),
        normalize_character_sex(template_value(template, &["sex", "Sex"], &EMPTY_TEXT)),
        infer_character_sex(&json!(name), &json!(voice_label)),
    ])
}

/// The ten persona fields resolved before any template text is formatted —
/// everything `agent.py:2825-2844` computes, plus the template they came from
/// so the list fields can be read from the same object.
struct Persona<'a> {
    name: String,
    locale: String,
    country: String,
    region: String,
    born: String,
    origin_detail: String,
    sex: Sex,
    voice_style: String,
    political_temperament: String,
    job: String,
    template: &'a Value,
}

impl Persona<'_> {
    /// The sex as the profile writes it: `""` only survives as `Sex::Unstated`
    /// never reaching a profile, because [`decided_sex`] ends at `unknown`.
    fn sex_name(&self) -> &'static str {
        self.sex.as_str()
    }
}

/// The persona fields, resolved the way `agent.py:2816-2844` resolves them.
///
/// `country_name` is the country half of the character id — the one name that
/// cannot drift out of step with the file it came from — and the template is
/// the character's own definition, or the default persona for a voice chosen
/// without one.
fn resolve_persona<'a>(
    voice: &Map<String, Value>,
    template: &'a Value,
    country_name: &str,
) -> Persona<'a> {
    let locale = or_fallback(
        normalize_locale(&or_text(&[
            voice.get("lang"),
            voice.get("culture"),
            voice.get("locale"),
        ])),
        "en-GB",
    );
    let name = or_fallback(
        template_text(template, &["name", "Name"], &json!("Greg")),
        "Greg",
    );
    let country = persona_country(template, country_name);
    let region = or_fallback(
        template_text(template, &["region", "Region"], &EMPTY_TEXT),
        country_name,
    );
    let born = template_text(template, &["born", "Born"], &EMPTY_TEXT);
    let origin_detail = or_fallback(
        template_text(
            template,
            &["origin_detail", "OriginDetail"],
            &Value::String(region.clone()),
        ),
        &region,
    );
    let voice_label = or_text(&[voice.get("display_name"), voice.get("name")])
        .trim()
        .to_owned();
    let sex = persona_sex(voice, template, &name, &voice_label);

    Persona {
        name,
        locale,
        country,
        region,
        born,
        origin_detail,
        sex,
        voice_style: template_text(template, &["voice_style", "VoiceStyle"], &EMPTY_TEXT),
        political_temperament: template_text(
            template,
            &["political_temperament", "PoliticalTemperament"],
            &EMPTY_TEXT,
        ),
        job: template_text(template, &["job", "Job"], &EMPTY_TEXT),
        template,
    }
}

/// The character's country: the id's own country first, the authored field
/// behind it, and the English-speaking world behind both (`agent.py:2822`).
fn persona_country(template: &Value, country_name: &str) -> String {
    let authored = template_text(template, &["country", "Country"], &EMPTY_TEXT);
    first_non_empty(&[country_name, &authored], "the English-speaking world")
}

/// The persona's text fields, each published under its plain key and its
/// title-case alias (`agent.py:2845-2864`): the country files were authored
/// across both styles, and `{HomeRegion}` is the alias of `{region}` that
/// keeps older templates resolving.
fn persona_fact_pairs<'p>(persona: &'p Persona<'_>) -> Vec<(&'static str, &'p str)> {
    vec![
        ("name", persona.name.as_str()),
        ("Name", persona.name.as_str()),
        ("locale", persona.locale.as_str()),
        ("Locale", persona.locale.as_str()),
        ("region", persona.region.as_str()),
        ("Region", persona.region.as_str()),
        ("country", persona.country.as_str()),
        ("Country", persona.country.as_str()),
        ("born", persona.born.as_str()),
        ("Born", persona.born.as_str()),
        ("home_region", persona.region.as_str()),
        ("HomeRegion", persona.region.as_str()),
        ("origin_detail", persona.origin_detail.as_str()),
        ("OriginDetail", persona.origin_detail.as_str()),
        ("voice_style", persona.voice_style.as_str()),
        ("VoiceStyle", persona.voice_style.as_str()),
        (
            "political_temperament",
            persona.political_temperament.as_str(),
        ),
        (
            "PoliticalTemperament",
            persona.political_temperament.as_str(),
        ),
        ("job", persona.job.as_str()),
        ("Job", persona.job.as_str()),
    ]
}

/// `format_values` (`agent.py:2845`): the kwargs every persona string is
/// formatted with.
///
/// Each fact appears twice — `{name}` and `{Name}` — because the country files
/// were authored across both styles, and `{HomeRegion}` is an alias of
/// `{region}` left in place so older templates still resolve
/// (`agent.py:2856`). The titled copies are Python's `str.title()`, which
/// capitalises word-wise rather than uppercasing.
fn format_values(persona: &Persona, pronouns: &Pronouns) -> Map<String, Value> {
    let pronoun = |key: &str| pronouns.get(key).map_or_else(String::new, py_str);
    let subject = pronoun("subject_pronoun");
    let object = pronoun("object_pronoun");
    let adjective = pronoun("possessive_adjective");
    let possessive = pronoun("possessive_pronoun");
    let reflexive = pronoun("reflexive_pronoun");
    let sex = persona.sex_name();

    let mut values = Map::new();
    for (key, text) in persona_fact_pairs(persona) {
        values.insert(key.to_owned(), Value::String(text.to_owned()));
    }
    for (key, text) in [
        ("sex", sex.to_owned()),
        ("Sex", py_title(sex)),
        ("subject_pronoun", subject.clone()),
        ("SubjectPronoun", py_title(&subject)),
        ("object_pronoun", object.clone()),
        ("ObjectPronoun", py_title(&object)),
        ("possessive_adjective", adjective.clone()),
        ("PossessiveAdjective", py_title(&adjective)),
        ("possessive_pronoun", possessive.clone()),
        ("PossessivePronoun", py_title(&possessive)),
        ("reflexive_pronoun", reflexive.clone()),
        ("ReflexivePronoun", py_title(&reflexive)),
    ] {
        values.insert(key.to_owned(), Value::String(text));
    }
    values
}

/// The persona's prose, formatted and sex-adapted: the four fields
/// `agent.py:2881-2911` builds once `format_values` exists.
struct Prose {
    backstory_lines: Vec<Value>,
    mannerisms: Vec<Value>,
    speech_register: Vec<Value>,
    backstory: String,
}

/// `" ".join(lines)` over the list the source joins.
fn join_lines(lines: &[Value]) -> String {
    lines.iter().map(py_str).collect::<Vec<String>>().join(" ")
}

/// The character's `backstory_lines`, `mannerisms`, `speech_register`, and
/// `backstory`, each formatted against `values` and then adapted to the
/// character's sex.
///
/// `backstory` is adapted twice when it falls back to the joined lines: the
/// lines were already adapted as a list, and the source adapts the joined text
/// again (`agent.py:2910-2911`). That double pass is reproduced rather than
/// tidied away, because the second pass is what rewrites a line that the first
/// pass left alone. `speech_register` is deliberately never adapted — those
/// are wording rules ("me" for "my"), and rewriting pronouns inside one would
/// corrupt the rule (`agent.py:2894-2897`).
fn build_prose(persona: &Persona, values: &Map<String, Value>) -> Prose {
    let sex = json!(persona.sex_name());
    let list = |source: &Value| normalize_character_list(source, values);
    let authored = |keys: &[&str]| template_value(persona.template, keys, &EMPTY_LIST);

    let backstory_lines = adapt_character_list_for_sex(
        &build_backstory_lines(persona.template, values, default_backstory_lines()),
        &sex,
    );
    let mut mannerisms = list(authored(&["mannerisms", "Mannerisms"]));
    if mannerisms.is_empty() {
        mannerisms = list(template_value(
            default_template(),
            &["mannerisms", "Mannerisms"],
            &EMPTY_LIST,
        ));
    }
    let mannerisms = adapt_character_list_for_sex(&mannerisms, &sex);
    let speech_register = list(authored(&["speech_register", "SpeechRegister"]));

    let mut backstory = format_character_template_text(
        template_value(persona.template, &["backstory", "Backstory"], &EMPTY_TEXT),
        values,
    );
    if backstory.is_empty() {
        backstory = format_character_template_text(
            template_value(default_template(), &["backstory", "Backstory"], &EMPTY_TEXT),
            values,
        );
    }
    if backstory.is_empty() {
        backstory = join_lines(&backstory_lines);
    }
    let backstory = adapt_character_text_for_sex(&json!(backstory), &sex);

    Prose {
        backstory_lines,
        mannerisms,
        speech_register,
        backstory,
    }
}

impl Persona<'_> {
    /// The fact sheet for this persona: [`build_character_fact_sheet`] called
    /// with the persona's own fields, which is exactly what the profile
    /// builder does at `agent.py:2912`.
    fn fact_sheet(&self) -> Value {
        build_character_fact_sheet(&FactFields {
            name: json!(self.name),
            region: json!(self.region),
            country: json!(self.country),
            origin_detail: json!(self.origin_detail),
            sex: json!(self.sex_name()),
            voice_style: json!(self.voice_style),
            political_temperament: json!(self.political_temperament),
            job: json!(self.job),
        })
    }
}

/// The three voice fields the profile records (`agent.py:2957-2959`), in
/// `voice_name`, `voice_display_name`, `voice_provider` order.
///
/// These are `voice.get(key, "")`, not an `or` chain: a key authored as `null`
/// is present, so `str(None)` spells `"None"` — the one place in the builder
/// where an authored null survives as text rather than as the empty string.
/// `provider` falls through to `source`, whose own default is read first.
fn voice_text_fields(voice: &Map<String, Value>) -> [String; 3] {
    let present = |key: &str, default: &Value| py_str(voice.get(key).unwrap_or(default));
    [
        present("name", &EMPTY_TEXT).trim().to_owned(),
        present("display_name", &EMPTY_TEXT).trim().to_owned(),
        present("provider", voice.get("source").unwrap_or(&EMPTY_TEXT))
            .trim()
            .to_owned(),
    ]
}

/// The profile's `slug` (`agent.py:2938`): the name of its conversation folder.
///
/// A resolved character slugs from its id, which also carries the disambiguator
/// that keeps two same-named characters in one country apart. A browser voice
/// under a chosen country gets the same shape from its persona name. Only a
/// voice with no country at all — an unknown id, or a selection saved before
/// countries existed — keeps the name-and-locale folder.
fn profile_slug(
    catalog: &CharacterCatalog,
    identifier: &str,
    authored: bool,
    country_name: &str,
    name: &str,
    locale: &str,
) -> String {
    if authored {
        return catalog.character_folder(identifier);
    }
    if country_name.is_empty() {
        return format!("{}-{}", slugify_character_name(name), locale.to_lowercase());
    }
    slugify_character_name(&format!("{country_name}-{name}"))
}

/// The profile's `cache_id` (`agent.py:2930`): the country file's authored
/// prompt-cache id, empty when the file was written before ids were added or
/// this is the code's own persona.
///
/// `(definition or {}).get("id") or ""` — the same `or` the builder uses for
/// its other fields, so an authored `null` id is the empty string here rather
/// than the `"None"` a `dict.get` default would have spelled.
fn authored_cache_id(definition: Option<&Value>) -> String {
    definition
        .and_then(|found| found.get("id"))
        .map_or(String::new(), |id| or_text(&[Some(id)]).trim().to_owned())
}

impl CharacterCatalog {
    /// The voice the profile speaks with, the character id it was asked for,
    /// and that character's authored definition.
    ///
    /// `agent.py:2804-2814`. The id is the caller's, else the voice's own, and
    /// an authored character rewrites the voice's `name`, `provider`, and
    /// `character_id` because it owns both its persona and its voice file —
    /// the definition wins over whatever the browser claimed.
    fn resolve_voice(
        &self,
        selected_voice: Option<&Value>,
        character_id: Option<&Value>,
    ) -> (Map<String, Value>, Option<String>, Option<Value>) {
        let mut voice = match selected_voice {
            Some(Value::Object(fields)) => fields.clone(),
            _ => Map::new(),
        };
        let text = or_text(&[character_id, voice.get("character_id")])
            .trim()
            .to_owned();
        let id = (!text.is_empty()).then_some(text);
        let definition = self
            .character_definition(id.as_deref().unwrap_or(""))
            .filter(|found| truthy(Some(found)));

        if let (Some(identifier), Some(authored)) = (id.as_deref(), definition.as_ref()) {
            for (key, value) in [
                ("name", or_text(&[authored.get("voice"), voice.get("name")])),
                (
                    "provider",
                    or_text(&[authored.get("provider"), voice.get("provider")]),
                ),
                ("character_id", identifier.to_owned()),
            ] {
                voice.insert(key.to_owned(), Value::String(value.trim().to_owned()));
            }
        }
        (voice, id, definition)
    }

    /// The country the profile belongs to: the country half of the character
    /// id, else the one the browser says its voice belongs to
    /// (`agent.py:2821-2823`).
    fn voice_country(voice: &Map<String, Value>, identifier: &str) -> String {
        let (country, _key) = split_character_id(identifier);
        if country.is_empty() {
            return or_text(&[voice.get("country")]).trim().to_owned();
        }
        country
    }

    /// `build_character_profile` (`agent.py:2803`): the persona a conversation
    /// runs on, from the voice the browser picked and the character id that
    /// voice speaks.
    ///
    /// A voice with no character behind it is not an error: it gets the code's
    /// own persona ("Greg") and the country the browser claims for it, because
    /// a country file declares characters and nothing else.
    #[must_use]
    pub fn build_character_profile(
        &self,
        selected_voice: Option<&Value>,
        character_id: Option<&Value>,
    ) -> Value {
        let (voice, id, definition) = self.resolve_voice(selected_voice, character_id);
        let identifier = id.as_deref().unwrap_or("");
        let template = match definition {
            Some(ref found) => found,
            None => default_template(),
        };
        let country_name = Self::voice_country(&voice, identifier);
        let persona = resolve_persona(&voice, template, &country_name);
        let values = format_values(&persona, &pronouns_for_character_sex(persona.sex));
        let prose = build_prose(&persona, &values);
        let [voice_name, voice_display_name, voice_provider] = voice_text_fields(&voice);

        let extras = ProfileExtras {
            identifier,
            voice_name,
            voice_display_name,
            voice_provider,
            // The country file's authored prompt-cache id, empty when the file
            // predates ids or this is the code's own persona.
            cache_id: authored_cache_id(definition.as_ref()),
            slug: profile_slug(
                self,
                identifier,
                definition.is_some(),
                &country_name,
                &persona.name,
                &persona.locale,
            ),
        };
        profile_payload(self, &persona, prose, &extras)
    }
}

/// The profile fields that come from the voice and the country file rather
/// than from the persona: what the voice is called, who serves it, the id its
/// prompt block is named after, and the memory folder it owns.
struct ProfileExtras<'a> {
    identifier: &'a str,
    voice_name: String,
    voice_display_name: String,
    voice_provider: String,
    cache_id: String,
    slug: String,
}

/// The assembled profile document (`agent.py:2871-2900`): the persona's own
/// fields, the prose after sex adaptation, the fact sheet, and the voice and
/// identity fields in [`ProfileExtras`].
///
/// Key order here is the order the source writes them, which is also the order
/// a saved profile document shows a reader; the HTTP layer sorts them before
/// they reach the browser either way.
fn profile_payload(
    catalog: &CharacterCatalog,
    persona: &Persona<'_>,
    prose: Prose,
    extras: &ProfileExtras<'_>,
) -> Value {
    let mut profile = Map::new();
    for (key, value) in [
        ("name", json!(persona.name)),
        ("locale", json!(persona.locale)),
        ("character_id", json!(extras.identifier)),
        ("cache_id", json!(extras.cache_id)),
        (
            "voice_pitch",
            json!(catalog.character_baseline_pitch(extras.identifier)),
        ),
        ("slug", json!(extras.slug)),
        ("region", json!(persona.region)),
        ("country", json!(persona.country)),
        ("born", json!(persona.born)),
        ("origin_detail", json!(persona.origin_detail)),
        ("sex", json!(persona.sex_name())),
        ("voice_style", json!(persona.voice_style)),
        (
            "political_temperament",
            json!(persona.political_temperament),
        ),
        ("job", json!(persona.job)),
        ("mannerisms", Value::Array(prose.mannerisms)),
        ("speech_register", Value::Array(prose.speech_register)),
        ("backstory_lines", Value::Array(prose.backstory_lines)),
        ("backstory", json!(prose.backstory)),
        ("facts", persona.fact_sheet()),
        ("voice_name", json!(extras.voice_name)),
        ("voice_display_name", json!(extras.voice_display_name)),
        ("voice_provider", json!(extras.voice_provider)),
    ] {
        profile.insert(key.to_owned(), value);
    }
    Value::Object(profile)
}

/// `character_facts` (`agent.py:4690`): the fact sheet a profile carries, or
/// one rebuilt from its own fields for a profile saved before fact sheets were
/// written into them.
///
/// An authored-but-empty `facts` object is falsy in Python, so it rebuilds
/// rather than returning `{}` — which is what keeps a profile trimmed by an
/// older version of this app answering self-questions at all.
#[must_use]
pub fn character_facts(character_profile: Option<&Value>) -> Value {
    let profile = character_profile.unwrap_or(&Value::Null);
    if let Some(facts @ Value::Object(fields)) = profile.get("facts") {
        if !fields.is_empty() {
            return facts.clone();
        }
    }

    build_character_fact_sheet(&FactFields::from_profile(profile))
}

/// One of the prompt's bullet lists, as the strings it prints: a list's
/// elements, or — Python iterating a bare string — its characters, and nothing
/// for an absent key. Reuses the persona's [`iterable_lines`] because the
/// prompt reads the same profile fields the persona wrote.
fn prompt_list(profile: &Value, key: &str) -> Vec<String> {
    iterable_lines(profile.get(key).unwrap_or(&EMPTY_LIST))
        .iter()
        .map(py_str)
        .collect()
}

/// `"\n".join(f"- {item}" for item in items)` — the bullet block both the
/// mannerisms and the backstory summary take.
fn bullets(items: &[String], prefix: &str) -> String {
    items
        .iter()
        .map(|item| format!("{prefix}{item}"))
        .collect::<Vec<String>>()
        .join("\n")
}

/// The register block `build_character_system_prompt` interpolates: empty for
/// a character that authors no register, which is why characters without one
/// keep exactly the prompt they had before the block existed.
fn prompt_register_block(facts: &Value, rules: &[String]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    format!(
        concat!(
            "- Speech register (binding on your wording; where it conflicts with any ",
            "general instruction to sound calm, precise, or well-spoken, this wins):\n",
            "  Voice style to realise: {}\n",
            "{}\n"
        ),
        get_text(facts, "voice_style", ""),
        bullets(rules, "  - "),
    )
}

/// The seven standing instructions every prompt ends with
/// (`agent.py:3031-3037`), and the only part of the prompt a character cannot
/// change.
const PROMPT_RULES: &str = concat!(
    "- Treat the explicit persona facts above as canonical for direct factual self-questions.\n",
    "- Use the backstory and mannerisms primarily for flavor, framing, and tone.\n",
    "- Stay in this character consistently for the whole reply.\n",
    "- Use this character name when referring to yourself.\n",
    "- Do not refer to yourself as The Entity In The Shed unless the user is explicitly asking about the app.\n",
    "- Do not volunteer that you are AI, software, or a model unless the user asks that directly.\n",
    "- Keep the facts accurate, but let the character's background and attitude color the wording."
);

/// `build_character_system_prompt` (`agent.py:2992`): the persona written into
/// the model's system turn.
///
/// The facts come from [`character_facts`] rather than the profile's own
/// fields, so a profile saved before fact sheets were written still answers
/// self-questions in the same words a freshly built one would. Each line's
/// fallback is the source's own `dict.get` default, including the
/// `they`/`them`/`their` the pronoun line carries.
///
/// `name` and `locale` are indexed directly by the source, so a profile
/// missing them raises there; here they read as the empty string.
#[must_use]
pub fn build_character_system_prompt(character_profile: &Value) -> String {
    let facts = character_facts(Some(character_profile));

    // Only a character that authors a register gets this block, so characters
    // without one keep exactly the prompt they had. It is worded as a binding
    // rule because a described accent is otherwise read as background colour:
    // the model will happily write "informal, packed with rhyming slang" into
    // the profile and then answer in neutral, well-spoken English
    // (agent.py:2997-3001).
    let register_block =
        prompt_register_block(&facts, &prompt_list(character_profile, "speech_register"));

    format!(
        "{identity}{body}{rules}",
        identity = prompt_identity_block(character_profile, &facts),
        body = prompt_backstory_block(character_profile, &register_block),
        rules = PROMPT_RULES,
    )
}

/// The identity half of the system prompt: who the character is, ending with
/// the mannerisms the source lists last (`agent.py:2966-2985`).
///
/// The source writes the whole prompt as one f-string; splitting it at the
/// boundary the prompt itself draws — identity lines above, register and
/// backstory below — keeps each half readable without changing a byte of the
/// text the model is handed.
fn prompt_identity_block(profile: &Value, facts: &Value) -> String {
    let fact = |key: &str| get_text(facts, key, "");
    let field = |key: &str| get_text(profile, key, "");

    format!(
        concat!(
            "Active character profile:\n",
            "- Name: {}\n",
            "- Locale: {}\n",
            "- Region: {}\n",
            "- Country: {}\n",
            "- Born: {}\n",
            "- Sex: {}\n",
            "- Self pronouns: {}/{}/{}\n",
            "- Origin detail: {}\n",
            "- Job: {}\n",
            "- Voice style: {}\n",
            "- Political temperament: {}\n",
            "- Mannerisms:\n",
            "{}\n"
        ),
        field("name"),
        field("locale"),
        field("region"),
        field("country"),
        field("born"),
        get_text(facts, "sex", &get_text(profile, "sex", "unknown")),
        get_text(facts, "subject_pronoun", "they"),
        get_text(facts, "object_pronoun", "them"),
        get_text(facts, "possessive_adjective", "their"),
        fact("origin_detail"),
        fact("job"),
        fact("voice_style"),
        fact("political_temperament"),
        or_fallback(
            bullets(&prompt_list(profile, "mannerisms"), "- "),
            "- None recorded."
        ),
    )
}

/// The register block, then the two backstory sections (`agent.py:2986-2996`).
///
/// The register block carries its own trailing newline, which is why the
/// source's template writes no separator before `- Backstory summary`; the
/// character with no authored register contributes an empty string there and
/// the two sections sit flush against the mannerisms above.
fn prompt_backstory_block(profile: &Value, register_block: &str) -> String {
    format!(
        concat!(
            "{}",
            "- Backstory summary:\n",
            "{}\n",
            "- Detailed backstory:\n",
            "{}\n"
        ),
        register_block,
        bullets(&prompt_list(profile, "backstory_lines"), "- "),
        get_text(profile, "backstory", ""),
    )
}

/// `public_character_profile` (`agent.py:3041`): the profile as the browser
/// sees it.
///
/// The persona's internal fields — the template it was built from, the
/// `speech_register` rules the prompt consumes but the UI never shows — are
/// left out, and every key takes the source's own `dict.get` default so a
/// partial profile still serialises into a shape the client can read.
#[must_use]
pub fn public_character_profile(character_profile: &Value) -> Value {
    let field =
        |key: &str, fallback: Value| character_profile.get(key).cloned().unwrap_or(fallback);

    let mut profile = Map::new();
    for (key, value) in [
        ("name", field("name", Value::Null)),
        ("locale", field("locale", Value::Null)),
        ("character_id", field("character_id", json!(""))),
        ("region", field("region", json!(""))),
        ("country", field("country", json!(""))),
        ("born", field("born", json!(""))),
        ("origin_detail", field("origin_detail", json!(""))),
        ("sex", field("sex", json!("unknown"))),
        ("voice_style", field("voice_style", json!(""))),
        (
            "political_temperament",
            field("political_temperament", json!("")),
        ),
        ("job", field("job", json!(""))),
        ("mannerisms", field("mannerisms", json!([]))),
        ("backstory", field("backstory", json!(""))),
        ("backstory_lines", field("backstory_lines", json!([]))),
        ("facts", field("facts", json!({}))),
        ("slug", field("slug", Value::Null)),
    ] {
        profile.insert(key.to_owned(), value);
    }
    Value::Object(profile)
}

/// The default persona's `backstory` (`agent.py:2284`'s default templates).
///
/// It opens with `{name}`, so the persona builder's `str.format` pass is what
/// turns it into "Greg is an old itinerant raconteur…" — stored raw, exactly
/// as the country files store their own `{region}` prose.
const GREG_BACKSTORY: &str = r"{name} is an old itinerant raconteur of the English-speaking world, the sort of fellow who seems to have arrived from a railway platform that no longer appears on maps. He spent his young adulthood drifting between ports, market towns, back rooms above pubs, and municipal halls where debates ran late into the evening and the tea always seemed slightly over-brewed. In every place he learned the same lesson: people do not merely want facts, they want the shape of a thing, its weight, its smell, and the likely consequence of ignoring it. That habit made him an unusually attentive listener. He became known as the man who could hear a rambling tale, strip off the vanity and noise, and return the story clearer, sharper, and somehow more memorable than when it arrived. As the decades went on he collected the manners of half a dozen regions without ever quite belonging to one. He picked up courtroom precision from civil clerks, sea-wind fatalism from dockworkers, and a taste for theatrical phrasing from amateur actors who always believed the world could be improved by a stronger entrance. Age did not soften him into vagueness. If anything it made him stricter. He believes a useful answer should be direct, accurate, and seasoned with enough character that it remains in the mind after the words are gone. That is why he speaks as he does now: half practical guide, half weathered stage ghost, always trying to turn noise into something a sensible person can actually use.";
