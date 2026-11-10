//! The picker's view of a country: the names and labels it offers, the panel
//! it shows beside them, and the entries it mints.
//!
//! Ports the label, details and roster guards of `tests/test_characters.py`
//! and the `voice_human_name` cases of `tests/test_character_profiles.py`.

mod common;

use serde_json::json;
use shed_characters::identity::character_id;
use shed_characters::picker::{
    character_dedupe_key, character_details, character_label_marker, character_text_lines,
    disambiguate_labels, slugify_character_name, voice_human_name,
};
use shed_text::{voice_tier, VOICE_TIER_LEGACY};

use common::*;

/// A voice with only a ShortName: the display name is what the fixture
/// catalog adds, and the rule works without it.
fn named(short_name: &str) -> serde_json::Value {
    json!({ "name": short_name })
}

#[test]
fn a_voice_is_offered_under_a_human_name() {
    // `voice_human_name` (`agent.py:2707`): the locale prefix, the noise the
    // vendor spells into every product name, and the trailing "Neural" all
    // go, and what is left is the person's name.
    assert_eq!(voice_human_name(&named("en-GB-RyanNeural")), "Ryan");
    assert_eq!(
        voice_human_name(&named("fr-FR-RemyMultilingualNeural")),
        "Remy"
    );
    assert_eq!(
        voice_human_name(&named("en-IN-NeerjaExpressiveNeural")),
        "Neerja"
    );
    assert_eq!(voice_human_name(&named("ja-JP-NanamiNeural")), "Nanami");

    // The display name is tried first, and the vendor's own words are noise
    // in it too.
    assert_eq!(
        voice_human_name(&voice(
            "en-GB-RyanNeural",
            "Microsoft Ryan Online (Natural) - English (United Kingdom)",
            "Male",
            "en-GB"
        )),
        "Ryan"
    );
}

#[test]
fn a_voice_with_no_name_left_falls_back_to_the_short_name() {
    // Every word of the cleaned candidate can be noise or a single letter —
    // one-letter words are no name (`len(word) > 1`, `agent.py:2722`) — and
    // the picker still has to show something: the ShortName verbatim.
    assert_eq!(voice_human_name(&named("en-GB-X")), "en-GB-X");
    assert_eq!(
        voice_human_name(&json!({ "name": "", "display_name": "" })),
        ""
    );
}

#[test]
fn a_character_name_becomes_a_folder() {
    // `slugify_character_name` (`agent.py:2730`): filesystem-safe, and input
    // with nothing spellable in it still names a folder.
    assert_eq!(slugify_character_name("Reginald"), "reginald");
    assert_eq!(slugify_character_name("  Dr. Fu Manchu "), "dr-fu-manchu");
    assert_eq!(slugify_character_name("O'Brien"), "o-brien");
    assert_eq!(slugify_character_name("Rémy"), "r-my");
    assert_eq!(slugify_character_name("!!!"), "character");
}

#[test]
fn character_details_hold_a_backstory_written_as_one_string() {
    // `test_character_details_hold_a_backstory_written_as_one_string`
    // (`test_characters.py:478-483`): iterating that string is the failure
    // this guards — the panel would spell it out one character at a time.
    let details = character_details(&json!({ "backstory_lines": "One line.\nThen another." }));

    assert_eq!(
        details["backstory_lines"],
        json!(["One line.", "Then another."])
    );
}

#[test]
fn character_details_always_answer_in_the_same_shape() {
    // Every key is present, blank when unauthored (`agent.py:2533-2535`), so
    // the client renders one shape; and an entry that is not an object at all
    // still yields the panel, exactly as `{}` does in the source.
    for entry in [json!({}), json!("a string"), json!(null)] {
        let details = character_details(&entry);
        assert_eq!(
            details,
            json!({ "born": "", "job": "", "backstory_lines": [], "backstory": "" })
        );
    }

    // What the file wrote reaches the panel, trimmed.
    let details = character_details(&json!({
        "born": " 1954 ",
        "job": "Ledger keeper",
        "backstory": " Long account. ",
        "backstory_lines": ["One", " ", 5, "Two"],
    }));
    assert_eq!(
        details,
        json!({
            "born": "1954",
            "job": "Ledger keeper",
            "backstory": "Long account.",
            "backstory_lines": ["One", "5", "Two"],
        })
    );
}

#[test]
fn panel_lines_are_the_file_s_own_words() {
    // `character_text_lines` (`agent.py:2544`): a list's elements spelled, a
    // bare string's lines, and nothing for a number or an object.
    assert_eq!(
        character_text_lines(Some(&json!(["  A  ", "", "B"]))),
        ["A", "B"]
    );
    assert_eq!(
        character_text_lines(Some(&json!("A\r\nB\u{2028}C"))),
        ["A", "B", "C"]
    );
    assert!(character_text_lines(Some(&json!(7))).is_empty());
    assert!(character_text_lines(None).is_empty());
}

#[test]
fn colliding_labels_gain_the_tag_that_tells_them_apart() {
    // `disambiguate_labels` (`agent.py:2672`): Canada ships Siqiniq in both
    // Inuktitut scripts, neither with a region of its own.
    let mut entries = vec![
        json!({
            "label": "Siqiniq",
            "name": "iu-Cans-CA-SiqiniqNeural",
            "id": "Canada:Siqiniq",
            "character_name": "Siqiniq",
        }),
        json!({
            "label": "Siqiniq",
            "name": "iu-Latn-CA-SiqiniqNeural",
            "id": "Canada:Siqiniq-latn",
            "character_name": "Siqiniq",
        }),
        json!({
            "label": "Taqqiq",
            "name": "iu-Cans-CA-TaqqiqNeural",
            "id": "Canada:Taqqiq",
            "character_name": "Taqqiq",
        }),
    ];

    disambiguate_labels(&mut entries);

    let labels: Vec<&str> = entries
        .iter()
        .map(|entry| entry["label"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(labels, ["Siqiniq (Cans)", "Siqiniq (Latn)", "Taqqiq"]);
}

#[test]
fn a_marker_without_a_script_subtag_is_the_rest_of_the_key() {
    // `character_label_marker` (`agent.py:2656`): the voice usually tells
    // them apart; when it does not, the disambiguated part of the character
    // key does.
    assert_eq!(
        character_label_marker(&json!({
            "label": "Siqiniq",
            "name": "en-GB-RyanNeural",
            "id": "United Kingdom:Alpha-two",
            "character_name": "Alpha",
        })),
        "Two"
    );
    // With neither, the marker is the voice's name verbatim.
    assert_eq!(
        character_label_marker(&json!({
            "label": "Alpha",
            "name": "en-GB-RyanNeural",
            "id": "United Kingdom:Alpha",
            "character_name": "Alpha",
        })),
        "en-GB-RyanNeural"
    );
}

#[test]
fn a_dedupe_key_is_country_region_and_name() {
    // `character_dedupe_key` (`agent.py:2561`): lowercased and trimmed, so
    // the same character spelled two ways is one entry.
    assert_eq!(
        character_dedupe_key(" United Kingdom ", "Testshire", "Alpha"),
        "character|united kingdom|testshire|alpha"
    );
}

/// Port of `test_character_picker_shares_a_voice_without_collapsing_entries`
/// (`test_characters.py:171`): two characters on one voice file are two
/// options, not one — the dedupe key carries the file entry, not the voice.
#[test]
fn sharing_a_voice_is_not_a_duplicate() {
    let roster = roster_of_fixture(COUNTRY);
    assert_eq!(roster.len(), 2);
    assert_eq!(roster[0]["character_name"], json!("Alpha"));
    assert_eq!(roster[1]["character_name"], json!("Beta"));
    assert_eq!(roster[0]["name"], json!(SHARED_VOICE));
    assert_eq!(roster[1]["name"], json!(SHARED_VOICE));
    assert_ne!(roster[0]["id"], roster[1]["id"]);
    assert_ne!(roster[0]["dedupe_key"], roster[1]["dedupe_key"]);
    // Each keeps its own authored pitch rather than the voice's one depth.
    assert_eq!(roster[0]["pitch"], json!(ALPHA_PITCH));
    assert_eq!(roster[1]["pitch"], json!(BETA_PITCH));
}

#[test]
fn an_entry_describes_the_voice_then_the_person() {
    // The split at `agent.py:2640`: everything above the person fields
    // describes the voice, `source`/`kind`/`character_region`/`dedupe_key`/
    // `details` describe who the picker's info icon shows.
    let roster = roster_of_fixture(COUNTRY);
    let alpha = &roster[0];
    assert_eq!(alpha["id"], json!(character_id(COUNTRY, "Alpha")));
    // "name" is the voice ShortName the client hands straight back to speak.
    assert_eq!(alpha["name"], json!(SHARED_VOICE));
    assert_eq!(alpha["display_name"], fake_catalog()[0]["display_name"]);
    assert_eq!(alpha["label"], json!("Alpha - Testshire"));
    assert_eq!(alpha["character_region"], json!("Testshire"));
    assert_eq!(alpha["region"], json!(COUNTRY));
    assert_eq!(alpha["source"], json!("character"));
    assert_eq!(alpha["kind"], json!("character"));
    assert_eq!(
        alpha["dedupe_key"],
        json!("character|united kingdom|testshire|alpha")
    );
    // Authored fields win over the catalog's; the catalog fills the rest.
    assert_eq!(alpha["gender"], json!("male")); // not the catalog's "Male"
    assert_eq!(alpha["provider"], json!("edge"));
    assert_eq!(alpha["locale"], json!("en-GB"));
    assert_eq!(alpha["tier"], json!(1)); // "...SyntheticNeural" is neural
}

#[test]
fn an_unauthored_entry_still_offers_every_field() {
    // Python's `or ""` arms: a bare entry authors no region, pitch, sex or
    // person facts, and the entry still spells every key.
    let app = app_dir("entry-blanks");
    let entry = json!({ "name": "Gamma", "voice": SHARED_VOICE });
    write_country(&app, COUNTRY, &json!({ "characters": [entry.clone()] }));
    let voices = catalog_shipping(&[(format!("{COUNTRY}:Gamma"), entry)]);
    let roster = catalog(&app).country_roster(COUNTRY, &voices);
    assert_eq!(roster.len(), 1);
    let gamma = &roster[0];
    assert_eq!(gamma["label"], json!("Gamma")); // no region to append
    assert_eq!(gamma["character_region"], json!(""));
    assert_eq!(gamma["pitch"], json!(72.0)); // the default depth
    assert_eq!(gamma["gender"], json!("")); // neither file nor voice authored one
    for key in ["born", "job", "backstory"] {
        assert_eq!(gamma["details"][key], json!(""));
    }
    assert_eq!(gamma["details"]["backstory_lines"], json!([]));
}

#[test]
fn the_panel_shows_the_authored_file_not_a_profile() {
    // `details` come from the country file's own entry (`agent.py:2525`), in
    // the file's own words — not the profile's adapted account.
    let app = app_dir("panel");
    write_country(
        &app,
        COUNTRY,
        &json!({ "characters": [authored_character()] }),
    );
    let cat = catalog(&app);
    let roster = roster_for(&cat, COUNTRY);
    assert_eq!(
        roster[0]["details"],
        json!({
            "born": AUTHORED_BORN,
            "job": ALPHA_JOB,
            "backstory_lines": [AUTHORED_BACKSTORY_LINE, AUTHORED_BACKSTORY_SECOND_LINE],
            "backstory": AUTHORED_BACKSTORY,
        })
    );
}

/// Port of `test_character_roster_drops_unshipped_voices_and_says_so`
/// (`test_characters.py:661`): a character whose voice the catalog does not
/// ship is left out and named — a silent drop could not be told apart from
/// the character having been deleted.
#[test]
fn an_unshipped_voice_is_left_out_and_named() {
    let app = app_dir("unshipped");
    write_country(
        &app,
        COUNTRY,
        &json!({ "characters": [
            { "name": "Alpha", "voice": SHARED_VOICE },
            { "name": "Reginald", "voice": PHANTOM_VOICE },
        ] }),
    );
    let cat = catalog(&app);
    let voices = fake_catalog(); // ships SHARED and PLAIN, never the phantom
    let (roster, warnings) = capture_warnings(|| cat.country_roster(COUNTRY, &voices));
    assert_eq!(roster.len(), 1, "the phantom's character cannot be offered");
    assert_eq!(roster[0]["character_name"], json!("Alpha"));
    let named = warnings
        .iter()
        .find(|line| line.starts_with("character voices not shipped in United Kingdom:"))
        .unwrap_or_else(|| panic!("nothing named the unshipped voice: {warnings:?}"));
    assert!(named.contains("Reginald"), "{named}");
    assert!(named.contains(PHANTOM_VOICE), "{named}");
}

#[test]
fn a_country_offers_nothing_when_the_voice_lookup_failed() {
    // Python's falsy `voices` arm (`agent.py:2592-2596`): an empty catalog
    // is a failed lookup, not evidence — no entries, and no "not shipped"
    // warning blaming a voice for a lookup that never answered.
    let app = app_dir("lookup-failed");
    write_country(&app, COUNTRY, &two_on_one_voice());
    let cat = catalog(&app);
    let (roster, warnings) = capture_warnings(|| cat.country_roster(COUNTRY, &[]));
    assert!(roster.is_empty());
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_roster_offers_only_the_country_it_was_asked_for() {
    let app = app_dir("two-countries");
    write_country(
        &app,
        COUNTRY,
        &json!({ "characters": [{ "name": "Alpha", "voice": SHARED_VOICE }] }),
    );
    write_country(
        &app,
        "France",
        &json!({ "characters": [{ "name": "Denise", "voice": "fr-FR-SyntheticNeural" }] }),
    );
    let mut voices = fake_catalog();
    voices.push(voice(
        "fr-FR-SyntheticNeural",
        "Microsoft Denise",
        "Female",
        "fr-FR",
    ));
    let cat = catalog(&app);
    let france = cat.country_roster("France", &voices);
    assert_eq!(france.len(), 1);
    assert_eq!(france[0]["character_name"], json!("Denise"));
    assert_eq!(france[0]["region"], json!("France"));
    assert_eq!(cat.country_roster(COUNTRY, &voices).len(), 1);
}

#[test]
fn the_same_name_in_two_scripts_stays_two_options() {
    // Canada's authored Inuktitut pair: two entries named "Siqiniq" under one
    // region. The key suffix (`identity.rs`) keeps them two characters, and
    // the voice's script subtag is what the labels end up showing.
    let app = app_dir("inuktitut");
    write_country(
        &app,
        "Canada",
        &json!({ "characters": [
            { "name": "Siqiniq", "voice": "iu-Cans-CA-SyntheticNeural", "region": "Nunavut" },
            { "name": "Siqiniq", "voice": "iu-Latn-CA-SyntheticNeural", "region": "Nunavut" },
        ] }),
    );
    let voices = vec![
        voice("iu-Cans-CA-SyntheticNeural", "Siqiniq", "Female", "iu-Cans"),
        voice("iu-Latn-CA-SyntheticNeural", "Siqiniq", "Female", "iu-Latn"),
    ];
    let cat = catalog(&app);
    let roster = cat.country_roster("Canada", &voices);
    assert_eq!(roster.len(), 2);
    assert_ne!(roster[0]["dedupe_key"], roster[1]["dedupe_key"]);
    assert_ne!(roster[0]["id"], roster[1]["id"]);
    let labels: Vec<&str> = roster
        .iter()
        .map(|entry| entry["label"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        labels,
        ["Siqiniq - Nunavut (Cans)", "Siqiniq - Nunavut (Latn)"]
    );
}

#[test]
fn an_entry_tier_is_minted_from_the_catalog_voice() {
    // The entry carries no friendly_name and no description, so a tier read
    // from the entry alone would be lost (`agent.py:2632-2636`); read from
    // the catalog voice it survives, at every weight.
    let app = app_dir("tier");
    let multilingual = voice(
        "sv-SE-ElinMultilingualNeural",
        "Microsoft Elin Online (Natural) - Swedish",
        "Female",
        "sv-SE",
    );
    let legacy = voice("en-GB-SyntheticPlain", "Plain", "Male", "en-GB");
    write_country(
        &app,
        "Sweden",
        &json!({ "characters": [
            { "name": "Elin", "voice": "sv-SE-ElinMultilingualNeural" },
            { "name": "Plain", "voice": "en-GB-SyntheticPlain" },
        ] }),
    );
    let voices = vec![multilingual.clone(), legacy.clone()];
    let cat = catalog(&app);
    let roster = cat.country_roster("Sweden", &voices);
    assert_eq!(roster[0]["tier"], json!(voice_tier(&multilingual)));
    assert_eq!(roster[0]["tier"], json!(2));
    assert_eq!(roster[1]["tier"], json!(VOICE_TIER_LEGACY));
}
