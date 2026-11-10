//! The persona a conversation runs on: what a voice resolves to, what the
//! fact sheet and the system prompt carry, and which memory folder a profile
//! names.
//!
//! Ports the resolution guards of `tests/test_characters.py` (522-655) and
//! `tests/test_character_profiles.py`, plus the roster-vs-catalog invariants
//! (661-742) that run against the repository's real `characters/` data.

mod common;

use serde_json::{json, Value};
use shed_characters::identity::character_id;
use shed_characters::profile::{
    build_character_system_prompt, character_facts, default_character_templates,
    public_character_profile,
};
use std::collections::BTreeSet;

use common::*;

/// `_voice(SHARED_VOICE, "Microsoft Alpha Online (Natural)", "Male", "en-GB")`
/// (`test_characters.py:523`).
fn alpha_voice() -> Value {
    voice(
        SHARED_VOICE,
        "Microsoft Alpha Online (Natural)",
        "Male",
        "en-GB",
    )
}

/// `_voice(PLAIN_VOICE, "Microsoft Solo Online...", "Male", "en-GB")`
/// (`test_characters.py:610`).
fn solo_voice() -> Value {
    voice(
        PLAIN_VOICE,
        "Microsoft Solo Online (Natural) - English (United Kingdom)",
        "Male",
        "en-GB",
    )
}

/// `dict(voice, key=value)`: the voice payload the browser sends once it has
/// picked a country or memorised a character id.
fn with(voice: &Value, key: &str, value: Value) -> Value {
    let mut merged = voice.as_object().cloned().unwrap_or_default();
    merged.insert(key.to_owned(), value);
    Value::Object(merged)
}

/// The synthetic country patched by Python's `patch.object(agent,
/// "load_country", return_value=TWO_ON_ONE_VOICE)`: the fixture written as
/// the catalog's own file.
fn two_on_one() -> shed_characters::catalog::CharacterCatalog {
    let app = app_dir("profile-fixture");
    write_country(&app, COUNTRY, &two_on_one_voice());
    catalog(&app)
}

fn name_of(profile: &Value) -> String {
    profile["name"].as_str().unwrap_or_default().to_owned()
}

/// Port of `test_character_baseline_pitch_knows_characters_and_defaults_neutral`
/// (`test_characters.py:511-519`). The `None` arm is the empty id: Python's
/// falsy id names no character, exactly as `""` splits to nothing here.
#[test]
fn baseline_pitch_knows_characters_and_defaults_to_neutral() {
    let cat = two_on_one();
    assert_eq!(cat.character_baseline_pitch(SHARED_ID), ALPHA_PITCH);
    assert_eq!(cat.character_baseline_pitch(PITCHED_ID), BETA_PITCH);
    // A plain voice id, an unknown id and a missing id are all neutral.
    assert_eq!(cat.character_baseline_pitch(PLAIN_VOICE), 72.0);
    assert_eq!(cat.character_baseline_pitch("nonsense"), 72.0);
    assert_eq!(cat.character_baseline_pitch(""), 72.0);
}

/// Port of `test_build_character_profile_resolves_a_character`
/// (`test_characters.py:522-532`).
#[test]
fn a_character_id_resolves_to_the_authored_person() {
    let profile =
        two_on_one().build_character_profile(Some(&alpha_voice()), Some(&json!(PITCHED_ID)));
    assert_eq!(profile["name"], json!("Beta"));
    assert_eq!(profile["slug"], json!("united-kingdom-beta"));
    assert_eq!(profile["voice_pitch"], json!(BETA_PITCH));
    assert_eq!(profile["character_id"], json!(PITCHED_ID));
    // The character owns the voice file, not whatever the client claimed.
    assert_eq!(profile["voice_name"], json!(SHARED_VOICE));
}

/// Port of `test_build_character_profile_reads_the_character_id_from_the_voice_payload`
/// (`test_characters.py:535-539`): the browser may send the id inside the
/// voice instead of beside it.
#[test]
fn a_character_id_rides_inside_the_voice_payload() {
    let voice = with(&alpha_voice(), "character_id", json!(PITCHED_ID));
    let profile = two_on_one().build_character_profile(Some(&voice), None);
    assert_eq!(profile["name"], json!("Beta"));
}

/// Port of `test_build_character_profile_carries_the_authored_job`
/// (`test_characters.py:542-550`).
#[test]
fn the_authored_job_travels_on_the_fact_sheet() {
    let profile =
        two_on_one().build_character_profile(Some(&alpha_voice()), Some(&json!(PITCHED_ID)));
    assert_eq!(profile["job"], json!(BETA_JOB));
    // It travels on the fact sheet too, which is what the prompt reads.
    assert_eq!(profile["facts"]["job"], json!(BETA_JOB));
    assert_eq!(character_facts(Some(&profile))["job"], json!(BETA_JOB));
}

/// Port of `test_the_system_prompt_states_the_job` (`test_characters.py:553-560`).
#[test]
fn the_system_prompt_states_the_job() {
    let profile =
        two_on_one().build_character_profile(Some(&alpha_voice()), Some(&json!(SHARED_ID)));
    let prompt = build_character_system_prompt(&profile);
    assert!(prompt.contains(&format!("- Job: {ALPHA_JOB}")), "{prompt}");
}

/// Port of `test_a_profile_with_no_job_still_builds_a_prompt`
/// (`test_characters.py:563-581`): a conversation saved before the field
/// existed must still render, with the line empty rather than absent.
#[test]
fn a_profile_with_no_job_still_builds_a_prompt() {
    let legacy = json!({
        "name": "Legacy",
        "locale": "en-GB",
        "region": "Testshire",
        "country": "United Kingdom",
        "born": "1970",
        "sex": "male",
        "mannerisms": [],
        "backstory_lines": ["Legacy is from Testshire."],
        "backstory": "Legacy is from Testshire.",
        "facts": {},
    });

    assert_eq!(character_facts(Some(&legacy))["job"], json!(""));
    assert!(
        build_character_system_prompt(&legacy).contains("- Job: \n"),
        "the job line must render empty, not vanish"
    );
}

/// Port of `test_the_public_profile_carries_the_job` (`test_characters.py:584-591`).
#[test]
fn the_public_profile_carries_the_job() {
    let profile =
        two_on_one().build_character_profile(Some(&alpha_voice()), Some(&json!(SHARED_ID)));
    assert_eq!(public_character_profile(&profile)["job"], json!(ALPHA_JOB));
}

/// Port of `test_build_character_profile_without_a_character_id_uses_the_locale_template`
/// and `test_a_voice_with_no_country_keeps_the_name_and_locale_folder`
/// (`test_characters.py:609-617`, `641-645`): unchanged from before characters
/// existed — folder is name + locale, persona from the locale template.
#[test]
fn a_voice_with_no_character_or_country_names_its_folder_by_locale() {
    let real = catalog(&repo_root());
    let profile = real.build_character_profile(Some(&solo_voice()), None);
    assert_eq!(profile["character_id"], json!(""));
    assert_eq!(profile["voice_pitch"], json!(72.0));
    assert_eq!(
        profile["slug"],
        json!(format!("{}-en-gb", name_of(&profile).to_lowercase()))
    );
}

/// Port of `test_build_character_profile_ignores_an_unknown_character_id`
/// (`test_characters.py:620-628`): the key names no real character, so no
/// persona and no pitch come from it — but the country it names is honoured.
#[test]
fn an_unknown_character_id_names_its_country_but_no_person() {
    let real = catalog(&repo_root());
    let profile = real.build_character_profile(
        Some(&solo_voice()),
        Some(&json!(format!("{COUNTRY}:Nobody"))),
    );
    assert_eq!(profile["voice_pitch"], json!(72.0));
    let slug = profile["slug"].as_str().unwrap_or_default().to_lowercase();
    assert!(!slug.contains("nobody"), "{slug}");
    assert_eq!(
        profile["slug"],
        json!(format!(
            "united-kingdom-{}",
            name_of(&profile).to_lowercase()
        ))
    );
}

/// Port of `test_a_browser_voice_takes_the_folder_of_the_country_it_was_chosen_under`
/// (`test_characters.py:631-638`): no character id of ours, so the country
/// travels in the payload.
#[test]
fn a_browser_voice_takes_its_folder_from_the_country_it_was_chosen_under() {
    let real = catalog(&repo_root());
    let browser = with(
        &voice(
            "Google UK English Female",
            "Google UK English Female",
            "Female",
            "ja-JP",
        ),
        "country",
        json!("Japan"),
    );
    let profile = real.build_character_profile(Some(&browser), None);
    assert_eq!(profile["character_id"], json!(""));
    assert_eq!(
        profile["slug"],
        json!(format!("japan-{}", name_of(&profile).to_lowercase()))
    );
}

/// Port of `test_two_characters_sharing_a_voice_get_distinct_memory_folders`
/// (`test_characters.py:648-654`).
#[test]
fn two_characters_sharing_a_voice_get_distinct_memory_folders() {
    let cat = two_on_one();
    let alpha = cat.build_character_profile(Some(&alpha_voice()), Some(&json!(SHARED_ID)));
    let beta = cat.build_character_profile(Some(&alpha_voice()), Some(&json!(PITCHED_ID)));
    let both = [alpha, beta];
    let slugs: BTreeSet<&str> = both.iter().filter_map(|p| p["slug"].as_str()).collect();
    assert_eq!(
        slugs,
        BTreeSet::from(["united-kingdom-alpha", "united-kingdom-beta"])
    );
}

/// Port of `test_a_voice_without_a_character_id_falls_back_to_the_global_default`
/// (`test_character_profiles.py:29-43`): a bare voice has no authored persona,
/// so the code's own default stands — and the country it was chosen under no
/// longer changes the answer.
#[test]
fn a_bare_voice_gets_the_code_s_own_persona_whatever_country_it_had() {
    let real = catalog(&repo_root());
    let voice = voice(
        "fr-FR-VivienneMultilingualNeural",
        "Microsoft Vivienne Multilingual Online - Natural",
        "Female",
        "fr-FR",
    );
    let profile = real.build_character_profile(Some(&voice), None);
    assert_eq!(
        profile["name"],
        default_character_templates()["default"]["name"]
    );
    assert_eq!(profile["character_id"], json!(""));

    // Named as well as wired: a client that still sends a country gets the
    // same persona, not a country-specific one.
    let with_country = with(&voice, "country", json!("France"));
    assert_eq!(
        real.build_character_profile(Some(&with_country), None)["name"],
        profile["name"]
    );
}

/// Port of `test_a_country_character_resolves_its_own_persona`
/// (`test_character_profiles.py:46-53`), against the real `characters/` data.
#[test]
fn a_country_character_resolves_its_own_persona() {
    let real = catalog(&repo_root());
    let voice = voice(
        "fr-FR-RemyMultilingualNeural",
        "Microsoft Remy Multilingual Online - Natural",
        "Male",
        "fr-FR",
    );
    let profile = real.build_character_profile(Some(&voice), Some(&json!("France:Remy")));
    assert_eq!(profile["name"], json!("Remy"));
    assert_eq!(profile["sex"], json!("male"));
    assert_eq!(profile["locale"], json!("fr-FR"));
    assert_eq!(profile["slug"], json!("france-remy"));
}

/// Port of `test_a_speech_register_reaches_the_character_prompt_as_rules`
/// (`test_character_profiles.py:56-67`): without it reaching the prompt the
/// mode prompt's "sound calm, precise and direct" stands unopposed.
#[test]
fn a_speech_register_reaches_the_prompt_as_rules() {
    let real = catalog(&repo_root());
    let profile = real.build_character_profile(None, Some(&json!("United Kingdom:Eddie")));
    let rules = profile["speech_register"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!rules.is_empty(), "Eddie authors a register");

    let prompt = build_character_system_prompt(&profile);
    assert!(prompt.contains("Speech register"), "{prompt}");
    for rule in &rules {
        let text = rule.as_str().unwrap_or_default();
        assert!(
            prompt.contains(text),
            "register rule missing from the prompt: {text}"
        );
    }
}

/// Port of `test_a_character_without_a_speech_register_gets_no_register_block`
/// (`test_character_profiles.py:70-76`): every other character keeps exactly
/// the prompt it had, so the block is conditional, not rendered empty.
#[test]
fn a_character_without_a_register_gets_no_register_block() {
    let real = catalog(&repo_root());
    let profile = real.build_character_profile(None, Some(&json!("United Kingdom:Brian")));
    assert_eq!(profile["speech_register"], json!([]));
    assert!(
        !build_character_system_prompt(&profile).contains("Speech register"),
        "the block must not render empty"
    );
}

/// Port of `test_two_country_characters_on_the_same_locale_do_not_collide`
/// (`test_character_profiles.py:79-86`).
#[test]
fn two_country_characters_on_the_same_locale_do_not_collide() {
    let real = catalog(&repo_root());
    let voice = voice(
        "fr-FR-VivienneMultilingualNeural",
        "Microsoft Vivienne Multilingual Online - Natural",
        "Female",
        "fr-FR",
    );
    let remy = real.build_character_profile(Some(&voice), Some(&json!("France:Remy")));
    let vivienne = real.build_character_profile(Some(&voice), Some(&json!("France:Vivienne")));
    assert_eq!(remy["name"], json!("Remy"));
    assert_eq!(vivienne["name"], json!("Vivienne"));
    assert_ne!(remy["slug"], vivienne["slug"]);
}

/// Port of `test_build_character_profile_prefers_voice_gender_for_uncurated_voice`
/// (`test_character_profiles.py:89-94`): a male voice with no curated profile
/// must never mis-gender to the locale template's sex (fr-FR -> female).
#[test]
fn an_uncurated_voice_keeps_its_own_gender() {
    let real = catalog(&repo_root());
    let voice = voice(
        "fr-FR-UnknownNeural",
        "Microsoft Unknown Online - Natural",
        "Male",
        "fr-FR",
    );
    let profile = real.build_character_profile(Some(&voice), None);
    assert_eq!(profile["sex"], json!("male"));
}

/// Port of `test_roster_offers_every_character_the_country_declares`
/// (`test_characters.py:661-667`): the picker sees exactly the file's
/// characters — the stage's exit gate, run against the real `characters/` data.
#[test]
fn the_roster_offers_every_character_the_country_declares() {
    let real = catalog(&repo_root());
    let roster = roster_for(&real, COUNTRY);
    let offered: BTreeSet<String> = roster
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(str::to_owned))
        .collect();
    let declared: BTreeSet<String> = real
        .character_keys(COUNTRY)
        .iter()
        .map(|(key, _)| character_id(COUNTRY, key))
        .collect();
    assert_eq!(offered, declared);
    assert!(roster.iter().all(|entry| entry["kind"] == "character"));
    assert!(roster.iter().all(|entry| entry["source"] == "character"));
}

/// Port of `test_roster_labels_are_unique` (`test_characters.py:730-732`).
#[test]
fn roster_labels_are_unique() {
    let real = catalog(&repo_root());
    let roster = roster_for(&real, COUNTRY);
    let labels: Vec<&str> = roster
        .iter()
        .map(|entry| entry["label"].as_str().unwrap_or_default())
        .collect();
    let unique: BTreeSet<&str> = labels.iter().copied().collect();
    assert_eq!(labels.len(), unique.len(), "{labels:?}");
}

/// Ports of `test_roster_labels_are_unique_even_for_same_named_characters`
/// and `test_same_named_characters_keep_distinct_picker_identities`
/// (`test_characters.py:711-727`), against the real Canada file.
#[test]
fn canada_s_scripts_stay_distinct_in_the_real_roster() {
    let real = catalog(&repo_root());
    let roster = roster_for(&real, "Canada");
    let labels: Vec<&str> = roster
        .iter()
        .map(|entry| entry["label"].as_str().unwrap_or_default())
        .collect();
    let unique: BTreeSet<&str> = labels.iter().copied().collect();
    assert_eq!(labels.len(), unique.len(), "{labels:?}");
    assert!(labels.contains(&"Siqiniq - Nunavut (Cans)"), "{labels:?}");
    assert!(labels.contains(&"Siqiniq - Nunavut (Latn)"), "{labels:?}");

    let dedupe = |wanted: &str| {
        roster
            .iter()
            .find(|entry| entry["id"] == json!(wanted))
            .map(|entry| entry["dedupe_key"].as_str().unwrap_or_default())
            .unwrap_or_default()
    };
    assert_ne!(dedupe("Canada:Siqiniq"), dedupe("Canada:Siqiniq-latn"));
}

/// Port of `test_roster_only_offers_the_country_it_was_asked_for`
/// (`test_characters.py:735-742`).
#[test]
fn a_real_roster_offers_only_the_country_it_was_asked_for() {
    let real = catalog(&repo_root());
    let uk = roster_for(&real, COUNTRY);
    let japan = roster_for(&real, "Japan");
    assert!(!uk.is_empty() && !japan.is_empty());
    let uk_regions: BTreeSet<&str> = uk
        .iter()
        .map(|entry| entry["region"].as_str().unwrap_or_default())
        .collect();
    let japan_regions: BTreeSet<&str> = japan
        .iter()
        .map(|entry| entry["region"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(uk_regions, BTreeSet::from([COUNTRY]));
    assert_eq!(japan_regions, BTreeSet::from(["Japan"]));
    let uk_ids: BTreeSet<&str> = uk.iter().filter_map(|entry| entry["id"].as_str()).collect();
    let japan_ids: BTreeSet<&str> = japan
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .collect();
    assert!(uk_ids.is_disjoint(&japan_ids));
}
