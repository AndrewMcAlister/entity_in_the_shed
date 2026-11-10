//! The LM Studio engine: the address its API answers on, how long one discovery
//! request waits, how a transient failure is retried, and where the model files
//! live on disk.
//!
//! Ported from `agent.py:393` (`LM_ENGINE_HOST`), `455` (`AI_MODEL_DISCOVERY_TIMEOUT`),
//! `491` (`LM_ENGINE_TRANSIENT_RETRIES`), `516-517` (the retry backoff),
//! `398-402` (the context-window sizes), `461` (`LM_ENGINE_REPLY_TIMEOUT`) and
//! `1312-1324` (`lmstudio_models_root`).
//!
//! Only the settings the shipped stages read are carried. The rest of the
//! `LM_ENGINE_*` block — `LM_ENGINE_KEEP_ALIVE`, the sampler temperatures,
//! `LM_ENGINE_STREAM_CHAT` — arrives with the stage that sends a chat request,
//! which is why this file is a slice of `agent.py`'s engine block rather than
//! the whole of it.

use std::path::PathBuf;

use crate::envsrc::EnvSource;
use crate::parse::{getenv_float, getenv_int, getenv_str, getenv_timeout_seconds};

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

/// `LM_ENGINE_NUM_CTX`'s fallback (`agent.py:398`): the token window assumed
/// for a model whose record does not carry one.
pub const DEFAULT_NUM_CTX: i64 = 32768;

/// `LM_ENGINE_LARGE_MODEL_CTX`'s fallback (`agent.py:401`): the window a model
/// over the size threshold below is capped to, so it stays within VRAM.
pub const DEFAULT_LARGE_MODEL_CTX: i64 = 16384;

/// `LM_ENGINE_LARGE_MODEL_SIZE_GB`'s fallback (`agent.py:402`): the file size
/// above which a model counts as large enough to need that cap.
pub const DEFAULT_LARGE_MODEL_SIZE_GB: f64 = 12.0;

/// `LM_ENGINE_REPLY_TIMEOUT`'s fallback (`agent.py:461`), in seconds, and
/// `None` is one of its legal values — the disable words turn the reply
/// watchdog off rather than setting it short.
pub const DEFAULT_REPLY_TIMEOUT_SECONDS: i64 = 500;

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
    /// `LM_ENGINE_NUM_CTX` (`agent.py:398`): the window `resolve_model_context_window`
    /// falls back to when a model's record carries none.
    ///
    /// The source reads the three context settings with a bare `int(os.getenv(...))`,
    /// so a value that does not parse is a `ValueError` at import — the app does not
    /// start. The clone reads them like every other setting: unparseable means the
    /// documented default, and a negative means zero rather than a negative window
    /// (`docs/PARITY.md`, "Structural changes").
    pub num_ctx: i64,
    /// `LM_ENGINE_LARGE_MODEL_CTX` (`agent.py:401`): the cap applied to a model
    /// whose file is bigger than [`LmEngine::large_model_size_gb`] gigabytes.
    pub large_model_ctx: i64,
    /// `LM_ENGINE_LARGE_MODEL_SIZE_GB` (`agent.py:402`), compared against the
    /// byte count a discovery payload reported.
    pub large_model_size_gb: f64,
    /// `LM_ENGINE_REPLY_TIMEOUT` (`agent.py:461`): the reply watchdog the page
    /// is told about as `chat_request_timeout_ms`. `None` — the disable words,
    /// or the source's own falsy branch — says the page waits without a timer.
    pub reply_timeout_seconds: Option<i64>,
}

impl LmEngine {
    /// Read the nine settings. Each read is written on its own line with its name
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
            num_ctx: getenv_int(src, "LM_ENGINE_NUM_CTX", DEFAULT_NUM_CTX),
            large_model_ctx: getenv_int(src, "LM_ENGINE_LARGE_MODEL_CTX", DEFAULT_LARGE_MODEL_CTX),
            large_model_size_gb: getenv_float(
                src,
                "LM_ENGINE_LARGE_MODEL_SIZE_GB",
                DEFAULT_LARGE_MODEL_SIZE_GB,
            ),
            reply_timeout_seconds: getenv_timeout_seconds(
                src,
                "LM_ENGINE_REPLY_TIMEOUT",
                Some(DEFAULT_REPLY_TIMEOUT_SECONDS),
            ),
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
    fn the_three_context_settings_default_to_the_sources_own_numbers() {
        let engine = LmEngine::from_source(&src(&[]));
        assert_eq!(engine.num_ctx, DEFAULT_NUM_CTX);
        assert_eq!(engine.large_model_ctx, DEFAULT_LARGE_MODEL_CTX);
        assert_eq!(engine.large_model_size_gb, DEFAULT_LARGE_MODEL_SIZE_GB);
        assert_eq!(
            engine.reply_timeout_seconds,
            Some(DEFAULT_REPLY_TIMEOUT_SECONDS)
        );
    }

    #[test]
    fn a_context_window_is_read_as_written_and_a_negative_is_floored() {
        let engine = LmEngine::from_source(&src(&[
            ("LM_ENGINE_NUM_CTX", "131072"),
            ("LM_ENGINE_LARGE_MODEL_CTX", "8192"),
            ("LM_ENGINE_LARGE_MODEL_SIZE_GB", "24.5"),
        ]));
        assert_eq!(engine.num_ctx, 131_072);
        assert_eq!(engine.large_model_ctx, 8_192);
        assert_eq!(engine.large_model_size_gb, 24.5);

        // `agent.py:398` keeps a negative (`int()` does not clamp), which would plan a
        // request with a negative window. The clone reads it through `getenv_int`,
        // whose floor is 0 — the same answer no-window produces everywhere it is read
        // (`docs/PARITY.md`, "Structural changes").
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_NUM_CTX", "-1")])).num_ctx,
            0
        );
    }

    #[test]
    fn an_unreadable_context_setting_is_the_documented_default_not_a_startup_crash() {
        // `int(os.getenv("LM_ENGINE_NUM_CTX", "32768"))` raises on `big`, and it runs at
        // import time, so the source's service never boots. The clone keeps the
        // documented default and starts (`docs/PARITY.md`, "Structural changes").
        let engine = LmEngine::from_source(&src(&[
            ("LM_ENGINE_NUM_CTX", "big"),
            ("LM_ENGINE_LARGE_MODEL_CTX", "16k"),
            ("LM_ENGINE_LARGE_MODEL_SIZE_GB", "twelve"),
        ]));
        assert_eq!(engine.num_ctx, DEFAULT_NUM_CTX);
        assert_eq!(engine.large_model_ctx, DEFAULT_LARGE_MODEL_CTX);
        assert_eq!(engine.large_model_size_gb, DEFAULT_LARGE_MODEL_SIZE_GB);
    }

    #[test]
    fn the_reply_timeout_names_its_disable_words() {
        // `LM_ENGINE_REPLY_TIMEOUT` is the one setting here that can mean "no timer",
        // and it is the one the page is handed back as `chat_request_timeout_ms`, so
        // the words that mean it are contract text.
        for word in [
            "",
            "0",
            "none",
            "false",
            "off",
            "disable",
            "disabled",
            "infinite",
            "unlimited",
        ] {
            assert_eq!(
                LmEngine::from_source(&src(&[("LM_ENGINE_REPLY_TIMEOUT", word)]))
                    .reply_timeout_seconds,
                None,
                "{word:?} means the page waits as long as the engine takes"
            );
        }

        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_REPLY_TIMEOUT", "30")])).reply_timeout_seconds,
            Some(30)
        );
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_REPLY_TIMEOUT", "1")])).reply_timeout_seconds,
            Some(1),
            "the floor is the source's own max(1, …)"
        );
        assert_eq!(
            LmEngine::from_source(&src(&[("LM_ENGINE_REPLY_TIMEOUT", "later")]))
                .reply_timeout_seconds,
            Some(DEFAULT_REPLY_TIMEOUT_SECONDS),
            "garbage falls back to the default rather than to no timer at all"
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
