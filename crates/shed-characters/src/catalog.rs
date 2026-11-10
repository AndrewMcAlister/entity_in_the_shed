//! The country files on disk: the index, the per-file caches, and the two
//! validators those caches run on every load.
//!
//! Ported from `agent.py:2308` (`country_files`), `agent.py:2344`
//! (`country_from_filename`), `agent.py:2352` (`country_list`), `agent.py:2357`
//! (`load_country`), `agent.py:2413` (`validate_character_pitches`),
//! `agent.py:2468` (`validate_character_voices`), `agent.py:2493`
//! (`character_keys`), `agent.py:2498` (`character_definition`), `agent.py:2506`
//! (`character_folder`), and `agent.py:2514` (`character_baseline_pitch`).
//!
//! The Python module keeps its two caches as globals (`agent.py:942`'s
//! `COUNTRY_INDEX_CACHE`, `agent.py:948`'s `COUNTRY_FILE_CACHE`) behind
//! functions that take no directory; the clone keeps them inside
//! [`CharacterCatalog`] so a test can point a catalog at a temporary folder
//! instead of the shipped `characters/`. The server builds one from
//! `AppConfig::base_dir` — `CHARACTER_PROFILES_DIR` is `BASE_DIR / "characters"`
//! (`agent.py:349`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;
use tracing::warn;

use shed_text::{coerce_f64, coerce_voice_depth, py_float_repr, py_repr, py_str, text_or_empty};

use crate::identity::{character_keys_from_entries, split_character_id};
use crate::picker::slugify_character_name;

/// `CHARACTER_FILE_SUFFIX` (`agent.py:350`).
pub const CHARACTER_FILE_SUFFIX: &str = "_characters.json";

/// The folder the country files live in: `CHARACTER_PROFILES_DIR`
/// (`agent.py:349`) relative to the process's base directory.
#[must_use]
pub fn characters_dir(base_dir: &Path) -> PathBuf {
    base_dir.join("characters")
}

/// `COUNTRY_INDEX_CACHE` (`agent.py:942-946`): the last directory listing,
/// kept so a request reuses it until a file appears, vanishes, or changes.
///
/// The signature carries only each file's name, mtime and size — the whole
/// point is to answer "did the folder move" without opening a single file,
/// and a country's roster changes exactly when its file's contents do.
#[derive(Debug)]
struct CachedIndex {
    signature: Vec<(String, i64, i64)>,
    index: BTreeMap<String, PathBuf>,
}

/// `COUNTRY_FILE_CACHE` (`agent.py:948-953`): one parsed country file per
/// name, kept until that file's own mtime and size move.
///
/// A country's roster is read on almost every request and the file behind it
/// changes only when someone edits it by hand, so re-reading and re-parsing
/// it per request would be the picker's dominant cost for no benefit.
#[derive(Debug)]
struct CachedFile {
    signature: (i64, i64),
    characters: Vec<Value>,
}

/// The country files, read through the same two signature caches the Python
/// app keeps as module globals (`agent.py:942`, `agent.py:948`).
///
/// The caches are per-instance rather than process globals: Python's exist
/// because a worker serves requests one after another and would otherwise
/// re-scan the folder every time, and the clone wants the same reuse without
/// process-wide state — each instance owns its locks, so two catalogs over
/// two folders never see each other's entries, and a test builds one over a
/// temporary directory instead of reaching into a global.
///
/// Entries are validated as they leave the file cache, which is where the
/// source validates them too: `load_country` checks the file it just parsed
/// and caches the checked data, so problems are re-reported only when the
/// file actually changes and a request that hits the cache says nothing.
#[derive(Debug)]
pub struct CharacterCatalog {
    profiles_dir: PathBuf,
    index: Mutex<Option<CachedIndex>>,
    files: Mutex<HashMap<String, CachedFile>>,
}

impl CharacterCatalog {
    /// A catalog over one directory — [`characters_dir`]`(&config.base_dir)`
    /// for the shipped files, a temporary folder for tests. The directory is
    /// handed in rather than derived, the same way `shed-models` takes its
    /// cache paths, so nothing here has to know where the project lives.
    #[must_use]
    pub fn new(profiles_dir: PathBuf) -> Self {
        Self {
            profiles_dir,
            index: Mutex::new(None),
            files: Mutex::new(HashMap::new()),
        }
    }

    /// The directory this catalog scans.
    #[must_use]
    pub fn profiles_dir(&self) -> &Path {
        &self.profiles_dir
    }

    /// `country_files` (`agent.py:2308`): `{country: path}` for every country
    /// file, cached on the directory listing.
    ///
    /// The country's display name comes from the filename, so dropping a
    /// `*_characters.json` file into `characters/` is all it takes to add a
    /// country. A directory that cannot be scanned is a warning and an empty
    /// index — and, exactly as the source does, no cache entry: the next call
    /// tries again.
    ///
    /// An empty cached index is never believed either. A listing can come
    /// back empty because the folder genuinely holds no country files, and
    /// one transient scan that finds nothing must not keep the picker empty
    /// for as long as the signature holds; rescanning is a single
    /// `read_dir`, and the cache exists to skip the file reads, not the
    /// listing.
    #[must_use]
    pub fn country_files(&self) -> BTreeMap<String, PathBuf> {
        let mut signature: Vec<(String, i64, i64)> = Vec::new();
        let mut index = BTreeMap::new();

        match self.scan(&mut signature, &mut index) {
            Ok(()) => {}
            Err(error) => {
                warn!(
                    "characters: cannot scan {}: {error}",
                    self.profiles_dir.display()
                );
                return index;
            }
        }

        signature.sort();
        let mut cache = self.index.lock().unwrap_or_else(PoisonError::into_inner);
        // `and COUNTRY_INDEX_CACHE.get("index")`: an empty index is never
        // believed, so an empty directory is rescanned every call.
        if let Some(cached) = cache.as_ref() {
            if cached.signature == signature && !cached.index.is_empty() {
                return cached.index.clone();
            }
        }
        *cache = Some(CachedIndex {
            signature,
            index: index.clone(),
        });
        index
    }

    /// One pass of the directory listing. The `Err` arm is Python's `OSError`
    /// around `os.scandir` itself; an entry that fails to stat mid-scan is
    /// Python's inner `except OSError: continue`.
    fn scan(
        &self,
        signature: &mut Vec<(String, i64, i64)>,
        index: &mut BTreeMap<String, PathBuf>,
    ) -> Result<(), String> {
        let listing = std::fs::read_dir(&self.profiles_dir).map_err(|error| error.to_string())?;
        for entry in listing.flatten() {
            // `entry.is_file()` and `entry.stat()` both follow symlinks, which
            // is what `metadata` does; one call covers both Python calls.
            let Ok(metadata) = std::fs::metadata(entry.path()) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if !metadata.is_file() || !name.ends_with(CHARACTER_FILE_SUFFIX) {
                continue;
            }
            let country = country_from_filename(&name);
            if country.is_empty() {
                continue;
            }
            signature.push((
                name,
                mtime_ns(&metadata),
                i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            ));
            index.insert(country, self.profiles_dir.join(entry.file_name()));
        }
        Ok(())
    }

    /// `country_list` (`agent.py:2352`): every country the picker can offer,
    /// alphabetically.
    #[must_use]
    pub fn country_list(&self) -> Vec<String> {
        self.country_files().into_keys().collect()
    }

    /// `load_country` (`agent.py:2357`): one country's file, parsed. Never
    /// reads another country's file.
    ///
    /// The `{"characters": [...]}` wrapper the source returns is the `Vec`
    /// itself; the wrapper only ever held that one key.
    #[must_use]
    pub fn load_country(&self, country: &str) -> Vec<Value> {
        let name = country.trim();
        let Some(path) = self.country_files().get(name).cloned() else {
            return Vec::new();
        };
        let Ok(metadata) = std::fs::metadata(&path) else {
            return Vec::new();
        };
        let signature = (
            mtime_ns(&metadata),
            i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        );

        {
            let cache = self.files.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(cached) = cache.get(name) {
                if cached.signature == signature {
                    return cached.characters.clone();
                }
            }
        }

        let characters = self.read_validated(name, &path);
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                name.to_owned(),
                CachedFile {
                    signature,
                    characters: characters.clone(),
                },
            );
        characters
    }

    /// The parse, filter and validate of a file that is no longer cached
    /// (`agent.py:2379-2399`). Validation happens here, on the cache miss,
    /// exactly where the source does it: a request that hits the cache says
    /// nothing, and problems are re-reported only when the file moves.
    fn read_validated(&self, name: &str, path: &Path) -> Vec<Value> {
        let file_name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let mut characters: Vec<Value> = Vec::new();
        let loaded = self.read_country_file(&file_name, path);

        if let Some(Value::Object(map)) = loaded.as_ref() {
            if let Some(Value::Array(entries)) = map.get("characters") {
                characters = entries
                    .iter()
                    .filter(|entry| {
                        entry.is_object() && !text_or_empty(entry.get("voice")).trim().is_empty()
                    })
                    .cloned()
                    .collect();
            }
        }

        // The validation keys are "{country}:{entry name}" (`agent.py:2396`),
        // which is *not* the character id — an entry's `name` is the authored
        // display name, and a missing one spells "None" exactly as `format`
        // would spell `None`. The dict comprehension those keys are built in
        // collapses two same-named entries to one slot: the position of the
        // first, the entry of the last.
        let mut keyed: Vec<(String, Value)> = Vec::new();
        for entry in &characters {
            let display = entry.get("name").unwrap_or(&Value::Null);
            let key = format!("{name}:{}", py_str(display));
            match keyed.iter_mut().find(|(existing, _)| *existing == key) {
                Some(slot) => slot.1 = entry.clone(),
                None => keyed.push((key, entry.clone())),
            }
        }
        let problems = validate_character_pitches(&keyed);
        if !problems.is_empty() {
            warn!(
                "character problems in {file_name}:\n{}",
                problems.join("\n")
            );
        }
        characters
    }

    /// The read plus parse of `agent.py:2380-2384`: any failure is a warning
    /// naming the file and `None`, after which the country loads as empty.
    ///
    /// The line embeds the exception's own text — `JSONDecodeError`'s wording
    /// in Python, `serde_json`'s here; the file name and the shape of the line
    /// are the ported part.
    fn read_country_file(&self, file_name: &str, path: &Path) -> Option<Value> {
        match std::fs::read_to_string(path)
            .map_err(|error| error.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
        {
            Ok(loaded) => Some(loaded),
            Err(error) => {
                warn!("character file {file_name} could not be read: {error}");
                None
            }
        }
    }

    /// `character_keys` (`agent.py:2493`): every character a country's file
    /// declares, as the ordered `(key, entry)` list Python's dict holds.
    #[must_use]
    pub fn character_keys(&self, country: &str) -> Vec<(String, Value)> {
        character_keys_from_entries(&self.load_country(country))
    }

    /// `character_definition` (`agent.py:2498`): the authored definition for a
    /// character id, or `None` for anything else.
    #[must_use]
    pub fn character_definition(&self, id: &str) -> Option<Value> {
        let (country, key) = split_character_id(id);
        if country.is_empty() || key.is_empty() {
            return None;
        }
        self.character_keys(&country)
            .into_iter()
            .find(|(entry_key, _)| *entry_key == key)
            .map(|(_, definition)| definition)
    }

    /// `character_folder` (`agent.py:2506`): the conversation folder for a
    /// character: `"<country>-<character name>"`.
    #[must_use]
    pub fn character_folder(&self, id: &str) -> String {
        let (country, key) = split_character_id(id);
        if country.is_empty() || key.is_empty() {
            return String::new();
        }
        slugify_character_name(&format!("{country}-{key}"))
    }

    /// `character_baseline_pitch` (`agent.py:2514`): a character's normal-voice
    /// pitch; 72.0 when absent or unrecognised.
    #[must_use]
    pub fn character_baseline_pitch(&self, id: &str) -> f64 {
        match self.character_definition(id) {
            None => 72.0,
            Some(definition) => coerce_voice_depth(definition.get("pitch"), 72.0),
        }
    }
}

/// `country_from_filename` (`agent.py:2344`): the display name a country
/// file's filename declares.
#[must_use]
pub fn country_from_filename(filename: &str) -> String {
    let Some(stem) = filename.strip_suffix(CHARACTER_FILE_SUFFIX) else {
        return String::new();
    };
    stem.replace('_', " ").trim().to_owned()
}

/// `validate_character_pitches` (`agent.py:2413`): human-readable problems
/// with a "characters" section (empty == valid).
///
/// A character that owns its voice file outright should stay neutral; only
/// characters that share a voice file author a pitch, and those must differ.
///
/// The entries arrive as the ordered `(id, entry)` pairs Python's dict
/// iterates — `load_country` mints the ids, the roster passes its own keys.
#[must_use]
pub fn validate_character_pitches(characters: &[(String, Value)]) -> Vec<String> {
    let mut problems = Vec::new();
    // `by_voice.setdefault(...)` keeps first-appearance order; a Vec of groups
    // with a name -> group index map reproduces that exactly.
    let mut groups: Vec<(String, Vec<(String, Value)>)> = Vec::new();
    let mut group_of: HashMap<String, usize> = HashMap::new();

    for (id, entry) in characters {
        let Some(entry) = entry.as_object() else {
            problems.push(format!("{id}: entry must be an object"));
            continue;
        };
        let voice_name = text_or_empty(entry.get("voice")).trim().to_owned();
        if voice_name.is_empty() {
            problems.push(format!("{id}: missing 'voice' (an edge ShortName)"));
        }

        // `entry.get("pitch") is not None`: a JSON null reads as the missing
        // key, so an authored `null` pitch is not checked at all.
        if let Some(raw_pitch) = entry.get("pitch").filter(|pitch| !pitch.is_null()) {
            let numeric = coerce_f64(raw_pitch);
            if let Some(numeric) = numeric.filter(|n| !(25.0 <= *n && *n <= 130.0)) {
                problems.push(format!(
                    "{id}: pitch {} is outside 25-130",
                    py_float_repr(numeric)
                ));
            } else if numeric.is_none() {
                problems.push(format!("{id}: pitch {} is not numeric", py_repr(raw_pitch)));
            }
        }

        let index = match group_of.get(&voice_name) {
            Some(index) => *index,
            None => {
                groups.push((voice_name.clone(), Vec::new()));
                let index = groups.len() - 1;
                group_of.insert(voice_name.clone(), index);
                index
            }
        };
        groups[index].1.push((id.clone(), entry.clone().into()));
    }

    for (voice_name, members) in &groups {
        if voice_name.is_empty() || members.len() < 2 {
            // A character with a voice to itself may still author a pitch:
            // pitch is a per-character property now, and a country file may
            // reuse one voice across countries at different pitches.
            continue;
        }
        problems.extend(pitch_conflicts(voice_name, members));
    }

    problems
}

/// The `seen` dict of `agent.py:2453-2462`: two characters left with the
/// same effective pitch on one voice file would be spoken identically, so
/// the later one names the earlier in a problem line.
fn pitch_conflicts(voice_name: &str, members: &[(String, Value)]) -> Vec<String> {
    // `seen` is a float-keyed dict: exact equality, and NaN never collides
    // with itself in Python — a Vec compared with `==` says the same.
    // `if owner:` is Python's truthiness on the stored id: an empty one
    // reports no clash and loses its slot to the next member.
    let mut seen: Vec<(f64, &str)> = Vec::new();
    let mut problems = Vec::new();
    for (id, entry) in members {
        let effective = coerce_voice_depth(entry.get("pitch"), 72.0);
        match seen.iter().position(|(depth, _)| *depth == effective) {
            Some(index) if !seen[index].1.is_empty() => problems.push(format!(
                "{id} and {} both use effective pitch {} on {voice_name}; \
                 characters sharing a voice file need distinct pitches",
                seen[index].1,
                py_float_repr(effective)
            )),
            Some(index) => seen[index].1 = id,
            None => seen.push((effective, id)),
        }
    }
    problems
}

/// `validate_character_voices` (`agent.py:2468`): problems with authored
/// voices the catalog does not ship (empty == valid).
///
/// A character naming a voice the catalog does not ship cannot be synthesised,
/// so `country_roster` leaves it out of the picker. Leaving it out is right;
/// doing it silently is not, because a one-word typo in a country file then
/// reads as the character never having existed at all.
#[must_use]
pub fn validate_character_voices(
    characters: &[(String, Value)],
    available_voices: &[Value],
) -> Vec<String> {
    let shipped: HashSet<String> = available_voices
        .iter()
        .map(|voice| text_or_empty(voice.get("name")).trim().to_owned())
        .collect();
    let mut problems = Vec::new();

    for (id, entry) in characters {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let voice_name = text_or_empty(entry.get("voice")).trim().to_owned();
        if !voice_name.is_empty() && !shipped.contains(&voice_name) {
            problems.push(format!(
                "{id}: voice {} is not shipped by the live catalog, so the character \
                 is left out of the picker",
                py_repr(&Value::String(voice_name))
            ));
        }
    }

    problems
}

/// `st_mtime_ns`: nanoseconds since the Unix epoch, negative before it.
fn mtime_ns(metadata: &Metadata) -> i64 {
    match metadata.modified() {
        Ok(time) => time
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|delta| i64::try_from(delta.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or_else(|error| {
                -i64::try_from(error.duration().as_nanos()).unwrap_or(i64::MIN)
            }),
        // A platform without an mtime reads as a constant, which pins the
        // cache open the way a filesystem without nanosecond timestamps did
        // for Python: the signature still matches itself, so the file is
        // read once and kept.
        Err(_) => 0,
    }
}
