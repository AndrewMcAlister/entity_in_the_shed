//! The country files: how the catalog distills the folder it scans, validates
//! what it parses, and caches what it validated.
//!
//! Ports the file-level guards of `tests/test_characters.py` (the country
//! list, the pitch validators, and the two laziness spies).

mod common;

use std::path::Path;

use serde_json::{json, Value};
use shed_characters::catalog::{
    characters_dir, validate_character_pitches, validate_character_voices,
};
use shed_characters::identity::{character_cache_id, character_id};

use common::*;

#[test]
fn country_list_is_distilled_from_the_filenames() {
    // `test_country_list_is_distilled_from_the_filenames`
    // (`test_characters.py:130`): the picker's countries come from the names,
    // not the contents — a file that cannot even parse is still a country.
    let app = app_dir("list");
    for country in ["United Kingdom", "United States", "Japan"] {
        write_country(&app, country, &json!({ "characters": [] }));
    }
    write_text(&app, "notes.md", "not a character file");
    write_text(&app, "France_characters.json", "{ this is not json");

    assert_eq!(
        catalog(&app).country_list(),
        ["France", "Japan", "United Kingdom", "United States"]
    );
}

#[test]
fn the_profiles_folder_hangs_off_the_app_root() {
    // `characters_dir` (`agent.py:349`): the one place that knows where the
    // country files live.
    assert_eq!(
        characters_dir(Path::new("/srv/shed")),
        Path::new("/srv/shed/characters")
    );
}

#[test]
fn a_country_the_folder_does_not_hold_reads_as_empty() {
    let app = app_dir("absent");
    write_country(&app, COUNTRY, &two_on_one_voice());

    assert!(catalog(&app).load_country("Nowhere").is_empty());
}

#[test]
fn pitch_problems_name_the_character_and_the_rule() {
    // `test_character_file_problems_are_reported` (`test_characters.py:355-365`):
    // every line names the character and says what is wrong, in the file's own
    // order.
    let problems = validate_character_pitches(&[
        ("UK:Alpha".to_owned(), json!({ "pitch": 70.0 })),
        (
            "UK:Beta".to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": "high" }),
        ),
        ("UK:Gamma".to_owned(), json!("not an object")),
    ]);

    assert_eq!(
        problems,
        [
            "UK:Alpha: missing 'voice' (an edge ShortName)",
            "UK:Beta: pitch 'high' is not numeric",
            "UK:Gamma: entry must be an object",
        ]
    );
}

#[test]
fn a_pitch_outside_the_engine_range_is_a_problem() {
    let problems = validate_character_pitches(&[(
        "UK:Alpha".to_owned(),
        json!({ "voice": SHARED_VOICE, "pitch": 140.0 }),
    )]);

    assert_eq!(problems, ["UK:Alpha: pitch 140.0 is outside 25-130"]);
}

#[test]
fn characters_sharing_a_voice_need_distinct_pitches() {
    // The drop that used to be silent, in the other direction: two characters
    // left with the same effective pitch on one voice file would be spoken
    // identically, so the later one names the earlier.
    let problems = validate_character_pitches(&[
        (
            SHARED_ID.to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": ALPHA_PITCH }),
        ),
        (
            PITCHED_ID.to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": ALPHA_PITCH }),
        ),
    ]);

    assert_eq!(
        problems,
        [format!(
            "{PITCHED_ID} and {SHARED_ID} both use effective pitch 70.0 on {SHARED_VOICE}; \
             characters sharing a voice file need distinct pitches"
        )]
    );
}

#[test]
fn an_unauthored_pitch_is_the_neutral_one_the_group_compares_against() {
    // `entry.get("pitch") is not None` skips the range check for an authored
    // null, but the character still joins its voice's group at the neutral
    // depth — so a companion authored at exactly that depth is a clash.
    let problems = validate_character_pitches(&[
        ("UK:Alpha".to_owned(), json!({ "voice": SHARED_VOICE })),
        (
            "UK:Beta".to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": 72.0 }),
        ),
    ]);

    assert_eq!(
        problems,
        [
            "UK:Beta and UK:Alpha both use effective pitch 72.0 on en-GB-SyntheticNeural; \
         characters sharing a voice file need distinct pitches"
        ]
    );
}

#[test]
fn distinct_pitches_on_one_voice_are_not_a_problem() {
    let problems = validate_character_pitches(&[
        (
            SHARED_ID.to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": ALPHA_PITCH }),
        ),
        (
            PITCHED_ID.to_owned(),
            json!({ "voice": SHARED_VOICE, "pitch": BETA_PITCH }),
        ),
    ]);

    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn a_voice_the_catalog_does_not_ship_is_named_in_the_warning() {
    // `test_a_character_naming_an_unshipped_voice_is_reported` (`test_characters.py:390+`):
    // the character leaves the picker, and the log says so — a silent drop
    // cannot be told apart from the character having been deleted.
    let problems = validate_character_voices(
        &[("UK:Reginald".to_owned(), json!({ "voice": PHANTOM_VOICE }))],
        &fake_catalog(),
    );

    assert_eq!(
        problems,
        ["UK:Reginald: voice 'en-GB-BrianMultilingualNeural' is not shipped by the live catalog, \
         so the character is left out of the picker"]
    );
}

#[test]
fn a_country_request_validates_only_that_country_file() {
    // `test_a_country_request_reads_only_that_country_file`
    // (`test_characters.py:749-762`), read through what it proves: an eager
    // loader would parse the whole folder and warn about the broken file.
    let app = app_dir("lazy");
    write_country(
        &app,
        "Japan",
        &json!({ "characters": [{ "name": "Yuki", "voice": PLAIN_VOICE }] }),
    );
    write_text(&app, "Zimbabwe_characters.json", "{ this is not json");

    let (_, warnings) = capture_warnings(|| catalog(&app).country_roster("Japan", &[]));

    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_file_that_has_not_moved_is_not_validated_twice() {
    // `test_a_repeat_request_reads_no_file_at_all` (`test_characters.py:765-776`)
    // and the source's own comment on it: problems are re-reported only when
    // the file moves, so a page reload is not a log flood.
    let app = app_dir("cache");
    let clash = json!({
        "characters": [
            { "name": "Alpha", "voice": SHARED_VOICE, "pitch": 70.0 },
            { "name": "Beta", "voice": SHARED_VOICE, "pitch": 70.0 },
        ],
    });
    write_country(&app, COUNTRY, &clash);
    let catalog = catalog(&app);

    let (_, first) = capture_warnings(|| catalog.load_country(COUNTRY));
    let (_, second) = capture_warnings(|| catalog.load_country(COUNTRY));
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].starts_with("character problems in United_Kingdom_characters.json:"));
    assert!(second.is_empty(), "{second:?}");

    // The file moves: same problem, reported again.
    let mut moved = clash;
    moved["characters"][0]["region"] = json!("Testshire");
    write_country(&app, COUNTRY, &moved);
    let (_, third) = capture_warnings(|| catalog.load_country(COUNTRY));
    assert_eq!(third.len(), 1, "{third:?}");
}

#[test]
fn a_file_that_cannot_be_read_loads_as_empty_and_says_so() {
    // `read_country_file` (`agent.py:2380`): any failure is one warning naming
    // the file, after which the country loads as empty.
    let app = app_dir("broken");
    write_text(&app, "France_characters.json", "{ this is not json");
    let catalog = catalog(&app);

    let (entries, warnings) = capture_warnings(|| catalog.load_country("France"));

    assert!(entries.is_empty());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].starts_with("character file France_characters.json could not be read:"));
}

#[test]
fn every_character_file_entry_carries_its_runtime_id() {
    // `test_prompt_cache.py:168-179`: the authored ids are data; the runtime
    // must agree with every one of them. The prompt cache names a character's
    // block after the file's `id`, so an entry whose id drifted from its
    // identity would silently split into two blocks per character.
    let real = catalog(&repo_root());
    let countries = real.country_list();
    assert!(!countries.is_empty());

    let mut checked = 0_usize;
    for country in &countries {
        for (key, _) in real.character_keys(country) {
            let id = character_id(country, &key);
            let wanted = character_cache_id(&id);
            let entry = real.character_definition(&id);
            assert_eq!(
                entry
                    .as_ref()
                    .and_then(|e| e.get("id"))
                    .and_then(Value::as_str),
                Some(wanted.as_str()),
                "{country}:{key}"
            );
            checked += 1;
        }
    }
    assert!(checked > 300, "only checked {checked} entries");
}
