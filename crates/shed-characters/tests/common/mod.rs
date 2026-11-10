//! Shared fixtures for `shed-characters`' integration tests.
//!
//! Port of the fixtures the Python suites build (`test_characters.py:14-123`,
//! `test_character_profiles.py:20-30`). Each integration test binary compiles
//! its own copy of this module and uses a subset of the helpers, so the ones a
//! given binary does not call are not dead code.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use shed_characters::catalog::{characters_dir, CharacterCatalog};
use shed_text::text_or_empty;
// The workspace denies `unused_crate_dependencies`, which judges every test
// binary as its own crate: the library's own dependencies are linked in
// whether or not a given suite calls them.
use fancy_regex as _;
use sha2 as _;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};

/// The Python suites' country (`test_characters.py:13`).
pub const COUNTRY: &str = "United Kingdom";
/// Two characters on one voice file, at different pitches
/// (`test_characters.py:19-20`). Synthetic on purpose: no test may assert
/// that a real character uses a particular voice.
pub const SHARED_VOICE: &str = "en-GB-SyntheticNeural";
/// A voice with exactly one character on it (`test_characters.py:20`).
pub const PLAIN_VOICE: &str = "en-GB-SyntheticSoloNeural";
/// Declared once but shipped by no live catalog — the ShortName that
/// de-listed Reginald without a word (`test_characters.py:26-29`).
pub const PHANTOM_VOICE: &str = "en-GB-BrianMultilingualNeural";
/// `{COUNTRY}:Alpha` / `{COUNTRY}:Beta` (`test_characters.py:21-22`).
pub const SHARED_ID: &str = "United Kingdom:Alpha";
pub const PITCHED_ID: &str = "United Kingdom:Beta";
pub const ALPHA_PITCH: f64 = 70.0;
pub const BETA_PITCH: f64 = 65.0;
pub const ALPHA_JOB: &str = "Synthetic job for Alpha";
pub const BETA_JOB: &str = "Synthetic job for Beta";

/// The facts the info panel shows, authored as prose the way a country file
/// does (`test_characters.py:38-41`).
pub const AUTHORED_BORN: &str = "1954";
pub const AUTHORED_BACKSTORY_LINE: &str = "Alpha came from Testshire and stayed.";
pub const AUTHORED_BACKSTORY_SECOND_LINE: &str = "Alpha keeps the ledger of the village.";
pub const AUTHORED_BACKSTORY: &str = "Alpha's long account of the county, written in the file.";

/// The repository root: where `characters/` lives.
#[must_use]
pub fn repo_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

/// A fresh temp directory shaped like the app root: with the `characters/`
/// folder inside it.
#[must_use]
pub fn app_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "shed-characters-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(error) = std::fs::create_dir_all(dir.join("characters")) {
        panic!("cannot create {}: {error}", dir.display());
    }
    dir
}

/// Write one country file into an app dir, named the way the catalog lists them.
pub fn write_country(app: &Path, country: &str, file: &Value) {
    let path = app
        .join("characters")
        .join(format!("{}_characters.json", country.replace(' ', "_")));
    if let Err(error) = std::fs::write(&path, file.to_string()) {
        panic!("cannot write {}: {error}", path.display());
    }
}

/// Write one raw file into an app dir's `characters/` folder, under any name:
/// the way a country file is written, but also usable for a file that is not
/// valid JSON, or one the catalog must not read at all.
pub fn write_text(app: &Path, file_name: &str, text: &str) {
    let path = app.join("characters").join(file_name);
    if let Err(error) = std::fs::write(&path, text) {
        panic!("cannot write {}: {error}", path.display());
    }
}

/// A catalog over an app dir, the way the server builds one at boot.
#[must_use]
pub fn catalog(app: &Path) -> CharacterCatalog {
    CharacterCatalog::new(characters_dir(app))
}

/// The Python fixtures' `_voice` (`test_characters.py:80-88`): a catalog
/// entry in the shape the voice listing mints.
#[must_use]
pub fn voice(short_name: &str, display_name: &str, gender: &str, locale: &str) -> Value {
    json!({
        "name": short_name,
        "display_name": display_name,
        "gender": gender,
        "provider": "edge",
        "locale": locale,
        "culture": locale,
    })
}

/// `FAKE_CATALOG` (`test_characters.py:91-94`).
#[must_use]
pub fn fake_catalog() -> Vec<Value> {
    vec![
        voice(
            SHARED_VOICE,
            "Microsoft Alpha Online (Natural) - English (United Kingdom)",
            "Male",
            "en-GB",
        ),
        voice(
            PLAIN_VOICE,
            "Microsoft Solo Online (Natural) - English (United Kingdom)",
            "Male",
            "en-GB",
        ),
    ]
}

/// `TWO_ON_ONE_VOICE` (`test_characters.py:43-62`).
#[must_use]
pub fn two_on_one_voice() -> Value {
    json!({
        "characters": [
            {
                "name": "Alpha",
                "voice": SHARED_VOICE,
                "pitch": ALPHA_PITCH,
                "region": "Testshire",
                "sex": "male",
                "job": ALPHA_JOB,
            },
            {
                "name": "Beta",
                "voice": SHARED_VOICE,
                "pitch": BETA_PITCH,
                "region": "Testmere",
                "sex": "female",
                "job": BETA_JOB,
            },
        ],
    })
}

/// `AUTHORED_CHARACTER` (`test_characters.py:67-77`): one character with
/// every panel fact filled in.
#[must_use]
pub fn authored_character() -> Value {
    json!({
        "name": "Alpha",
        "voice": SHARED_VOICE,
        "pitch": ALPHA_PITCH,
        "sex": "male",
        "region": "Testshire",
        "born": AUTHORED_BORN,
        "job": ALPHA_JOB,
        "backstory_lines": [AUTHORED_BACKSTORY_LINE, AUTHORED_BACKSTORY_SECOND_LINE],
        "backstory": AUTHORED_BACKSTORY,
    })
}

/// `_locale_of` (`test_characters.py:97-99`).
#[must_use]
pub fn locale_of(short_name: &str) -> String {
    let parts: Vec<&str> = short_name.split('-').collect();
    if parts.len() >= 2 {
        parts[..2].join("-")
    } else {
        String::new()
    }
}

/// `catalog_shipping` (`test_characters.py:102-113`): a catalog shipping
/// every voice a country declares, whoever it names — built from the country
/// file rather than written out, so no test has to know which voice a
/// character uses. The phantom is never in it.
#[must_use]
pub fn catalog_shipping(entries: &[(String, Value)]) -> Vec<Value> {
    let mut voices: Vec<Value> = Vec::new();
    for (_id, entry) in entries {
        let name = text_or_empty(entry.get("voice")).trim().to_owned();
        if name.is_empty() {
            continue;
        }
        let known = voices
            .iter()
            .any(|item| item.get("name").and_then(Value::as_str) == Some(name.as_str()));
        if known {
            continue;
        }
        // `str(entry.get("sex") or "")` — the authored sex verbatim, untrimmed.
        let gender = text_or_empty(entry.get("sex"));
        voices.push(voice(&name, &name, &gender, &locale_of(&name)));
    }
    voices
}

/// `_roster` (`test_characters.py:116-117`): a real country file's roster,
/// shipped by a catalog built from that same file.
#[must_use]
pub fn roster_for(catalog: &CharacterCatalog, country: &str) -> Vec<Value> {
    let entries = catalog.character_keys(country);
    catalog.country_roster(country, &catalog_shipping(&entries))
}

/// `_roster_of_fixture` (`test_characters.py:120-123`): the roster for the
/// synthetic two-on-one-voice country.
#[must_use]
pub fn roster_of_fixture(country: &str) -> Vec<Value> {
    // The Python patches `load_country`; the equivalent here is the fixture
    // written as that country's file in a temp dir of its own.
    let app = app_dir("fixture-roster");
    write_country(&app, country, &two_on_one_voice());
    catalog(&app).country_roster(country, &fake_catalog())
}

thread_local! {
    static WARNINGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // `warn!("text {x}")` hands the visitor the formatted arguments,
            // whose `Debug` is exactly the line the app would print.
            self.message = format!("{value:?}");
        }
    }
}

/// Run `run` under a subscriber that records every `warn!`/`error!` line,
/// and return its value with the recorded lines — this port's `caplog`.
pub fn capture_warnings<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    struct Recorder;

    impl Subscriber for Recorder {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() <= Level::WARN
        }

        fn event(&self, event: &Event<'_>) {
            if event.metadata().level() <= &Level::WARN {
                let mut visitor = MessageVisitor {
                    message: String::new(),
                };
                event.record(&mut visitor);
                WARNINGS.with(|lines| lines.borrow_mut().push(visitor.message));
            }
        }

        // The recorder watches events only; spans carry nothing it stores.
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    WARNINGS.with(|lines| lines.borrow_mut().clear());
    let result = tracing::subscriber::with_default(Recorder, run);
    (
        result,
        WARNINGS.with(|lines| std::mem::take(&mut *lines.borrow_mut())),
    )
}
