//! How much the app says about itself, and where those lines go.
//!
//! Ported from `agent.py:11524` (`configure_logging`).
//!
//! The Python app sets the level on `app.logger` alone and leaves the root logger at
//! `WARNING`, so its own narration reaches the daily log while library chatter does not.
//! `app.logger` is one logger shared by every module; in Rust each crate emits under its
//! own target, so the same rule is a target prefix: every crate in this workspace is named
//! `shed-*`, and `tracing`'s default target for a crate is its name with `::`-separated
//! modules after it (`shed_http::routes`). One prefix covers them all.

use std::sync::Once;

use shed_config::LogLevel;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// The prefix that marks a line as this app's own, as described in the module comment.
const APP_TARGET_PREFIX: &str = "shed_";

/// What everything outside the app is allowed to say, which is the root logger's `WARNING`
/// in the Python app.
const LIBRARY_LEVEL: LevelFilter = LevelFilter::WARN;

/// `init` may be called by the binary, by a test, and by a smoke run. The global subscriber
/// can only be set once per process, and the second attempt is not an error worth reporting:
/// it means something already decided how this process logs.
static INIT: Once = Once::new();

/// Installs the process's log.
///
/// The lines go to stderr, which is what the Python app's `app.logger` handler writes to and
/// what the service host captures into the daily log file. ANSI colour is off because the
/// same lines land in a text file, where escape codes are damage rather than decoration.
pub fn init(level: LogLevel) {
    INIT.call_once(|| {
        let formatter =
            tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::io::stderr);
        let subscriber =
            tracing_subscriber::registry().with(formatter.with_filter(app_filter(level)));
        match subscriber.try_init() {
            Ok(()) => tracing::info!(
                "logging at {} (set LOG_LEVEL to change); app lines only",
                level.name()
            ),
            Err(err) => eprintln!("shed-logging: a log subscriber was already installed: {err}"),
        }
    });
}

/// The rule that decides which lines the app keeps: ours at the configured level, everyone
/// else at [`LIBRARY_LEVEL`].
fn app_filter(level: LogLevel) -> Targets {
    Targets::new().with_target(APP_TARGET_PREFIX, level_filter(level)).with_default(LIBRARY_LEVEL)
}

/// `LogLevel` to `LevelFilter`. This mapping lives here rather than in `shed-config` because
/// `LogLevel` is the app's spelling and `LevelFilter` is the logger's.
fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::Registry;

    /// A writer the fmt layer can be pointed at, so a test can read the lines the filter
    /// chose instead of only reasoning about what it would choose.
    #[derive(Clone, Debug)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            match self.0.lock() {
                Ok(mut lines) => {
                    lines.extend_from_slice(buf);
                    Ok(buf.len())
                }
                Err(_) => Err(io::Error::other("the capture buffer was poisoned")),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Runs `body` with the app's own filter installed for `level`, capturing every line that
    /// survives it.
    fn lines_that_survive(level: LogLevel, body: impl FnOnce()) -> String {
        let shared = Arc::new(Mutex::new(Vec::new()));
        let buffer_owner = Arc::clone(&shared);
        let formatter = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(move || Capture(Arc::clone(&buffer_owner)));
        let subscriber = Registry::default().with(formatter.with_filter(app_filter(level)));
        tracing::subscriber::with_default(subscriber, body);
        match shared.lock() {
            Ok(lines) => String::from_utf8_lossy(&lines).into_owned(),
            Err(_) => String::new(),
        }
    }

    fn speak() {
        tracing::info!(target: "shed_http::routes", "app info");
        tracing::debug!(target: "shed_http::routes", "app debug");
        tracing::info!(target: "hyper::server", "library info");
        tracing::warn!(target: "hyper::server", "library warning");
    }

    #[test]
    fn app_lines_are_kept_and_library_info_is_not() {
        let lines = lines_that_survive(LogLevel::Info, speak);
        assert!(lines.contains("app info"));
        assert!(lines.contains("library warning"));
        assert!(!lines.contains("app debug"));
        assert!(!lines.contains("library info"));
    }

    /// `configure_logging` raises only the app's level, so a quieter setting quiets the app
    /// and leaves the library lines at `WARNING` exactly as they were.
    #[test]
    fn a_quieter_level_quiets_the_app_only() {
        let lines = lines_that_survive(LogLevel::Error, speak);
        assert!(lines.contains("library warning"));
        assert!(!lines.contains("app info"));
    }

    /// The prefix is what makes one rule cover every crate, so it is the part worth pinning:
    /// a crate target with modules after it still counts as one of ours.
    #[test]
    fn every_workspace_crate_target_counts_as_the_apps_own() {
        let lines = lines_that_survive(LogLevel::Debug, || {
            tracing::debug!(target: "shed_config::dotenv", "config");
            tracing::debug!(target: "shed_chat::reply", "chat");
            tracing::debug!(target: "shed_tts::edge", "tts");
        });
        assert!(lines.contains("config"));
        assert!(lines.contains("chat"));
        assert!(lines.contains("tts"));
    }

    #[test]
    fn levels_map_to_the_logger_spelling() {
        assert_eq!(level_filter(LogLevel::Debug), LevelFilter::DEBUG);
        assert_eq!(level_filter(LogLevel::Info), LevelFilter::INFO);
        assert_eq!(level_filter(LogLevel::Warn), LevelFilter::WARN);
        assert_eq!(level_filter(LogLevel::Error), LevelFilter::ERROR);
    }

    /// `init` is the one place the global subscriber is set, and calling it twice is a normal
    /// thing to happen in a process that also runs tests.
    #[test]
    fn init_can_be_called_more_than_once() {
        init(LogLevel::Warn);
        init(LogLevel::Warn);
    }
}
