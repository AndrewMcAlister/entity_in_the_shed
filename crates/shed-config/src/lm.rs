//! The LM Studio engine: the address its API answers on, how long one discovery
//! request waits, how a transient failure is retried, and where the model files
//! live on disk.
//!
//! Ported from `agent.py:393` (`LM_ENGINE_HOST`), `455` (`AI_MODEL_DISCOVERY_TIMEOUT`),
//! `491` (`LM_ENGINE_TRANSIENT_RETRIES`), `516-517` (the retry backoff) and
//! `1312-1324` (`lmstudio_models_root`).
//!
//! Only the six settings Stage 2 reads are carried. The rest of the `LM_ENGINE_*`
//! block — `LM_ENGINE_KEEP_ALIVE`, the sampler temperatures, the context-window
//! sizes, `LM_ENGINE_STREAM_CHAT` — arrives with the stage that sends a chat
//! request, which is why this file is a slice of `agent.py`'s engine block rather
//! than the whole of it.

use std::path::PathBuf;

use crate::envsrc::EnvSource;
use crate::parse::{getenv_int, getenv_str};

/// `LM_ENGINE_HOST`'s fallback (`agent.py:393`): the address LM Studio answers on
/// when nobody has moved it.
pub const DEFAULT_ENGINE_HOST: &str = "http://localhost:1234";

/// `AI_MODEL_DISCOVERY_TIMEOUT`'s fallback (`agent.py:455`), in seconds.
///
/// It is a socket read timeout rather than a deadline for the whole exchange, so
/// this long is the most a single read — the model list, or one streamed token —
/// waits before the request is called failed. `MIN_DISCOVERY_TIMEOUT` is the
/// `max(1, …)` wrapped around the read.
pub const DEFAULT_DISCOVERY_TIMEOUT: u64 = 600;
pub const MIN_DISCOVERY_TIMEOUT: u64 = 1;

/// `LM_ENGINE_TRANSIENT_RETRIES`'s fallback (`agent.py:491`). It counts *re*tries,
/// so a request is made this many plus one times, and zero means no retrying at all.
pub const DEFAULT_TRANSIENT_RETRIES: u64 = 2;

/// `LM_ENGINE_RETRY_BASE_MS`'s fallback and its floor (`agent.py:516`, `max(50, …)`):
/// a wait shorter than 50 ms is a retry that has not given the engine time to
/// notice it failed.
pub const DEFAULT_RETRY_BASE_MS: u64 = 250;
pub const MIN_RETRY_BASE_MS: u64 = 50;

/// `LM_ENGINE_RETRY_JITTER_MS`'s fallback (`agent.py:517`). Its only clamp is
/// `max(0, …)`, which [`getenv_int`] already applies.
pub const DEFAULT_RETRY_JITTER_MS: u64 = 150;

/// The engine settings Stage 2 needs in order to reach `GET {host}/v1/models` and
/// retry it.
#[derive(Debug, Clone)]
pub struct LmEngine {
    /// The API root with its trailing slashes removed. `agent.py:393` rstrips `/`
    /// rather than trimming whitespace, so `http://h:1234//` becomes `http://h:1234`
    /// while a leading space stays exactly where it was written. A blank setting
    /// stays a blank host.
    pub host: String,
    pub discovery_timeout_seconds: u64,
    pub transient_retries: u64,
    pub retry_base_ms: u64,
    pub retry_jitter_ms: u64,
}

impl LmEngine {
    /// Read the five settings. Each read is written on its own line with its name
    /// spelled out, which is what lets the settings gate match a shipped read to
    /// the name it reads (`shed-gates/src/settings.rs`).
    #[must_use]
    pub fn from_source(src: &EnvSource) -> Self {
        let host = getenv_str(src, "LM_ENGINE_HOST", DEFAULT_ENGINE_HOST);
        let timeout = getenv_int(
            src,
            "AI_MODEL_DISCOVERY_TIMEOUT",
            DEFAULT_DISCOVERY_TIMEOUT as i64,
        );
        let retries = getenv_int(
            src,
            "LM_ENGINE_TRANSIENT_RETRIES",
            DEFAULT_TRANSIENT_RETRIES as i64,
        );
        let base = getenv_int(src, "LM_ENGINE_RETRY_BASE_MS", DEFAULT_RETRY_BASE_MS as i64);
        let jitter = getenv_int(
            src,
            "LM_ENGINE_RETRY_JITTER_MS",
            DEFAULT_RETRY_JITTER_MS as i64,
        );

        Self {
            host: host.trim_end_matches('/').to_string(),
            discovery_timeout_seconds: clamped(timeout, MIN_DISCOVERY_TIMEOUT),
            transient_retries: clamped(retries, 0),
            retry_base_ms: clamped(base, MIN_RETRY_BASE_MS),
            retry_jitter_ms: clamped(jitter, 0),
        }
    }
}

/// `max(FLOOR, …)` on top of what [`getenv_int`] already applied: a parsed value is
/// never below zero, an unparseable one is the documented default, and the floor is
/// the clamp the call site adds — `max(1, …)` for the timeout (`agent.py:455`),
/// `max(50, …)` for the backoff (`516`) and `max(0, …)` for the retry count
/// (`491`), which is a floor that changes nothing here but names what the source
/// wrote.
///
/// The widening from `i64` is the clone's own. A negative can only reach here from
/// a negative default, and none of these have one; if one ever did, it lands on the
/// floor rather than wrapping into a wait that outlives the process.
fn clamped(value: i64, floor: u64) -> u64 {
    u64::try_from(value).unwrap_or(0).max(floor)
}

/// Where LM Studio keeps its model files, which is the root the disk scan walks
/// (`discover_lm_studio_models_from_disk`, `agent.py:1327`).
#[derive(Debug, Clone)]
pub struct LmStudio {
    /// `None` when neither `LMSTUDIO_MODELS_DIR` nor `USERPROFILE` names a directory
    /// to look in, which is the same case `agent.py:1322-1323` reports as `None`.
    pub models_dir: Option<PathBuf>,
}

impl LmStudio {
    /// `lmstudio_models_root`: the configured directory, or the default install
    /// layout, or nothing.
    ///
    /// The source reads both names with `os.environ.get` inside the function, which
    /// is the scattered read `How_To_Code` principle 19 forbids; here they are read
    /// once, here. `USERPROFILE` is a name the machine sets rather than the
    /// operator, so it is in `OS_PROVIDED` in the settings gate and deliberately not
    /// in `.env.example`.
    #[must_use]
    pub fn from_source(src: &EnvSource) -> Self {
        Self {
            models_dir: models_root(
                &getenv_str(src, "LMSTUDIO_MODELS_DIR", ""),
                &getenv_str(src, "USERPROFILE", ""),
                &getenv_str(src, "HOME", ""),
            ),
        }
    }
}

/// The resolution `lmstudio_models_root` performs, written apart from the reads so
/// the fall-back cases are testable on a machine that does set `USERPROFILE`.
fn models_root(configured: &str, user_profile: &str, home: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Some(expand_user(configured, user_profile.trim(), home.trim()));
    }

    let user_profile = user_profile.trim();
    if user_profile.is_empty() {
        return None;
    }
    Some(PathBuf::from(user_profile).join(".lmstudio").join("models"))
}

/// `Path.expanduser()`: a leading `~` becomes the home directory, and anything else
/// is the path as written.
///
/// `ntpath.expanduser` looks for `USERPROFILE` first and only then `HOME`, which is
/// the order kept here, and a path with no home to expand into is left alone rather
/// than guessed at.
fn expand_user(path: &str, user_profile: &str, home: &str) -> PathBuf {
    if path != "~" && !path.starts_with("~/") && !path.starts_with("~\\") {
        return PathBuf::from(path);
    }

    let directory = if user_profile.is_empty() {
        home
    } else {
        user_profile
    };
    if directory.is_empty() {
        return PathBuf::from(path);
    }

    let rest = path[1..].trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        PathBuf::from(directory)
    } else {
        PathBuf::from(directory).join(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn src(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_file_values(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn the_engine_settings_default_to_lms_studio_own_address_and_a_short_retry_ladder() {
        // None of the six names is set in a test process, so the fallbacks show
        // through — the same precondition `MCP defaults` already relies on.
        let engine = LmEngine::from_source(&src(&[]));
        assert_eq!(engine.host, DEFAULT_ENGINE_HOST);
        assert_eq!(engine.discovery_timeout_seconds, DEFAULT_DISCOVERY_TIMEOUT);
        assert_eq!(engine.transient_retries, DEFAULT_TRANSIENT_RETRIES);
        assert_eq!(engine.retry_base_ms, DEFAULT_RETRY_BASE_MS);
        assert_eq!(engine.retry_jitter_ms, DEFAULT_RETRY_JITTER_MS);
    }

    #[test]
    fn a_configured_engine_is_used_as_given() {
        let engine = LmEngine::from_source(&src(&[
            ("LM_ENGINE_HOST", "http://127.0.0.1:8080"),
            ("AI_MODEL_DISCOVERY_TIMEOUT", "30"),
            ("LM_ENGINE_TRANSIENT_RETRIES", "0"),
            ("LM_ENGINE_RETRY_BASE_MS", "1000"),
            ("LM_ENGINE_RETRY_JITTER_MS", "0"),
        ]));
        assert_eq!(engine.host, "http://127.0.0.1:8080");
        assert_eq!(engine.discovery_timeout_seconds, 30);
        assert_eq!(engine.transient_retries, 0);
        assert_eq!(engine.retry_base_ms, 1000);
        assert_eq!(engine.retry_jitter_ms, 0);
    }

    #[test]
    fn trailing_slashes_are_removed_and_a_blank_host_is_a_blank_host() {
        // A present-but-empty value is not an absent name, so `LM_ENGINE_HOST=` is an
        // empty address rather than the default, exactly as `agent.py:393` produces it.
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_HOST", "http://h:1234//")])).host,
            "http://h:1234"
        );
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_HOST", "http://h:1234/")])).host,
            "http://h:1234"
        );
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_HOST", "")])).host,
            "",
            "an empty value is what the operator asked for"
        );
    }

    #[test]
    fn the_two_floors_raise_a_number_that_is_too_small() {
        // `max(1, …)` and `max(50, …)` are the source's clamps. A zero here would mean
        // a request with no read timeout at all, and a retry that does not wait.
        let small = LmEngine::from_source(&src(&[
            ("AI_MODEL_DISCOVERY_TIMEOUT", "0"),
            ("LM_ENGINE_RETRY_BASE_MS", "1"),
        ]));
        assert_eq!(small.discovery_timeout_seconds, MIN_DISCOVERY_TIMEOUT);
        assert_eq!(small.retry_base_ms, MIN_RETRY_BASE_MS);

        let broken = LmEngine::from_source(&src(&[
            ("AI_MODEL_DISCOVERY_TIMEOUT", "soon"),
            ("LM_ENGINE_TRANSIENT_RETRIES", "-3"),
        ]));
        assert_eq!(broken.discovery_timeout_seconds, DEFAULT_DISCOVERY_TIMEOUT);
        assert_eq!(
            broken.transient_retries, 0,
            "getenv_int clamps a parsed value at zero"
        );
    }

    #[test]
    fn a_configured_models_directory_is_used_and_a_leading_tilde_is_expanded() {
        assert_eq!(
            models_root(" D:\\Models ", "", ""),
            Some(PathBuf::from("D:\\Models"))
        );
        assert_eq!(
            models_root("~/models", "C:\\Users\\AndrewBot", ""),
            Some(PathBuf::from("C:\\Users\\AndrewBot\\models"))
        );
        assert_eq!(
            models_root("~", "", "/home/andrew"),
            Some(PathBuf::from("/home/andrew"))
        );
        assert_eq!(
            models_root("~", "", ""),
            Some(PathBuf::from("~")),
            "no home to expand into"
        );
        assert_eq!(
            models_root("~/models", "C:\\Users\\AndrewBot", "/home/andrew"),
            Some(PathBuf::from("C:\\Users\\AndrewBot\\models")),
            "USERPROFILE wins, as it does for ntpath.expanduser"
        );
    }

    #[test]
    fn the_default_layout_is_lms_studio_own_models_folder_under_the_user_profile() {
        assert_eq!(
            models_root("", "C:\\Users\\AndrewBot", ""),
            Some(PathBuf::from("C:\\Users\\AndrewBot\\.lmstudio\\models"))
        );

        // `agent.py:1322-1323`: no configured directory and no user profile means
        // nothing to scan.
        assert_eq!(models_root("", "", ""), None);
        assert_eq!(models_root("   ", "  ", ""), None, "blank is unset here");
    }

    #[test]
    fn the_models_directory_read_falls_back_to_the_user_profile_of_the_machine() {
        // The file value shadows the process environment, so this is the configured
        // branch; the fall-back branch reads the real USERPROFILE and is exercised by
        // the boot smoke test rather than here.
        let studio = LmStudio::from_source(&src(&[("LMSTUDIO_MODELS_DIR", r"D:\Models")]));
        assert_eq!(studio.models_dir, Some(PathBuf::from(r"D:\Models")));
    }
}
