//! How much the app says about itself.
//!
//! Ported from `agent.py:11539` (`configure_logging`'s level lookup).

use crate::env::Env;

/// The five levels the app can be set to, named the way Python's `logging` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// `getattr(logging, os.getenv("LOG_LEVEL", "INFO").strip().upper(), logging.INFO)`, with
    /// one correction the Python version does not need.
    ///
    /// `getattr` accepts anything that is a *level number*, and `logging.NOTSET` is one: it is
    /// zero, which is not a level at all but the value that means "inherit from the root", so
    /// a `LOG_LEVEL` of `NOTSET` actually leaves the app listening at the root's `WARNING`.
    /// Python's own `isValidType` is what keeps a name like `logging.raiseExceptions` out of
    /// the level, and the fixed spelling list here does the same job.
    pub fn from_setting(env: &Env) -> Self {
        match env.get("LOG_LEVEL").unwrap_or_default().trim().to_uppercase().as_str() {
            "DEBUG" => Self::Debug,
            "WARNING" | "WARN" | "NOTSET" => Self::Warn,
            "ERROR" | "CRITICAL" | "FATAL" => Self::Error,
            // Anything else, including a blank setting, is the Python default.
            _ => Self::Info,
        }
    }

    /// The name `logging.getLevelName` would print, which is what the startup line quotes.
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(value: Option<&str>) -> LogLevel {
        let pairs =
            value.map(|v| vec![("LOG_LEVEL".to_string(), v.to_string())]).unwrap_or_default();
        LogLevel::from_setting(&Env::with_values(pairs))
    }

    #[test]
    fn the_default_is_info_and_the_name_is_case_insensitive() {
        assert_eq!(level(None), LogLevel::Info);
        assert_eq!(level(Some("")), LogLevel::Info);
        assert_eq!(level(Some("debug")), LogLevel::Debug);
        assert_eq!(level(Some("  WARNING  ")), LogLevel::Warn);
        assert_eq!(level(Some("error")), LogLevel::Error);
    }

    #[test]
    fn a_word_that_is_not_a_level_leaves_info_in_place() {
        // The Python lookup falls back to `logging.INFO` for anything that is not a level,
        // which is what a typo in `.env` should do: keep the app talking at the default rather
        // than silence it.
        assert_eq!(level(Some("verbose")), LogLevel::Info);
        assert_eq!(level(Some("raiseExceptions")), LogLevel::Info);
    }

    #[test]
    fn notset_means_the_root_level_not_the_quietest_one() {
        assert_eq!(level(Some("NOTSET")), LogLevel::Warn);
    }

    #[test]
    fn the_printed_names_are_python_names() {
        assert_eq!(LogLevel::Warn.name(), "WARNING");
        assert_eq!(LogLevel::Info.name(), "INFO");
        assert_eq!(LogLevel::Debug.name(), "DEBUG");
        assert_eq!(LogLevel::Error.name(), "ERROR");
    }
}
