//! The `/api/` routes Stage 3 serves, and the two shapes they answer in.
//!
//! Every handler here is a Flask view plus `@api_json_boundary`
//! (`agent.py:1002-1012`), which in the source means "any exception becomes
//! `internal server error` at 500 with the request id in front of the log
//! line". The clone has no exceptions to catch, and it does not pretend
//! otherwise: each helper these routes call — `country_list`,
//! `list_available_voices`, `country_roster`, `discover_lm_studio_models` —
//! is total, because every one of them logs and returns the empty value where
//! the Python original raises. So the `except` arms of these six views
//! (`agent.py:8489`, `8501`, `8533`, `8562`, `11603`) have no trigger here,
//! and neither do they in the source, whose own helpers swallow their
//! failures one level down (`agent.py:1308`, `2340`, `3229`). `docs/PARITY.md`
//! carries that as the Stage 3 row, and the envelope `api_error_response`
//! builds (`agent.py:987-999`) joins the first route that can actually reach
//! it — Stage 4's `chat_job_not_found` (`agent.py:9409`).
//!
//! What Stage 3 does carry of the hook is the part that is live: the
//! `X-Request-ID` every `/api/` response gets, which [`crate::request_id`]
//! puts there and every handler below can read.

mod models;
mod picker;

pub use models::{last_model_index, models, refresh_models, set_last_model_index};
pub use picker::{characters, countries, voices};

use serde::Deserialize;

/// The query parameters these routes read, which is all of
/// `request.args.get(...)` across the six views.
///
/// An unknown key is ignored and a repeated key takes the last value, where
/// Flask's `.get()` would take the first; the frozen client sends each name
/// once, and a browser that sends one twice has no defined behaviour to match.
#[derive(Debug, Default, Deserialize)]
pub(super) struct Args {
    /// `api_characters` (`agent.py:8525`): the country to load, empty for all.
    #[serde(default)]
    pub(super) country: Option<String>,
    /// `api_voices`/`api_characters` (`agent.py:8498`, `8524`): the browser's
    /// `navigator.language`, collapsed by `normalize_browser_language`.
    #[serde(default)]
    pub(super) language: Option<String>,
    /// `api_get_last_model_index` (`agent.py:8705`): answer as plain text.
    #[serde(default, rename = "text_report_format")]
    pub(super) text_report_format: Option<String>,
}

impl Args {
    /// `str(request.args.get(key) or "").strip()`: absent, blank-after-trim, or
    /// the trimmed value. `api_characters` (`agent.py:8525`) is the view this
    /// spelling comes from, and `api_countries` has no argument to read.
    fn trimmed(&self, value: &Option<String>) -> String {
        value.as_deref().unwrap_or_default().trim().to_owned()
    }

    /// `normalize_browser_language(request.args.get("language"))`
    /// (`agent.py:8498`, `8524`). `None` reads as `""`, which is what Python
    /// passes, and `normalize_browser_language` turns both into the default
    /// language.
    fn language(&self) -> String {
        shed_text::normalize_browser_language(self.language.as_deref().unwrap_or(""))
    }
}

/// `parse_bool(request.args.get("text_report_format"), default=False)`
/// (`agent.py:8705`), the one argument of the last-model-index pair that is
/// not a string.
fn text_report_format(args: &Args) -> bool {
    shed_config::parse::parse_bool(args.text_report_format.clone(), false)
}
