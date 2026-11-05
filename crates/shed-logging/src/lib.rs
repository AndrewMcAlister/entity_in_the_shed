//! stderr logging, the clone of `configure_logging` (`agent.py:11524–11550`).
//!
//! Python raised only `app.logger` to `LOG_LEVEL` and set `propagate =
//! False`, leaving library chatter at the root logger's WARNING ("a page
//! load does not bury the speech lines under request logs"). The Rust clone
//! mirrors that with one filter: events whose target starts with `shed_`
//! pass at the configured level, everything else (axum, hyper, tower) only
//! at WARN.
//!
//! `RUST_LOG` is deliberately never read — the Python app had exactly one
//! logging knob (`LOG_LEVEL`) and the operator's `.env` is the single source
//! of truth. The subscriber writes to stderr with no ANSI colours, because
//! the Windows service host appends the child's stderr to the daily log
//! file, where escape codes would be noise.
//!
//! One accepted mapping difference: `tracing` has no level above ERROR, so
//! a `CRITICAL` setting shares the ERROR filter floor (Python's CRITICAL
//! also admitted ERROR records only through `logger.critical`, which the
//! clone logs with `error!`).

use tracing::Level;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use shed_config::LogLevel;

/// Install the global subscriber and emit the startup line. A second call
/// is a clean error, not a panic.
pub fn install(level: LogLevel) -> Result<(), String> {
    let subscriber = tracing_subscriber::Registry::default()
        .with(filter_fn(move |meta| {
            permits(level, meta.target(), *meta.level())
        }))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        );
    subscriber
        .try_init()
        .map_err(|error| format!("cannot install the logging subscriber: {error}"))?;
    // Python logged this through app.logger at INFO, so a LOG_LEVEL above
    // INFO suppresses it here too — the filter already decides.
    tracing::info!("{}", startup_line(level));
    Ok(())
}

/// The startup line, contract text: the Python wording verbatim.
#[must_use]
pub fn startup_line(level: LogLevel) -> String {
    format!(
        "Logging at {} (set LOG_LEVEL to change); app lines only",
        level.label()
    )
}

/// Crate targets are the crate name with dashes as underscores, so every
/// app crate (`shed_config`, `shed_http`, …) starts with `shed_`.
#[must_use]
fn is_app_target(target: &str) -> bool {
    target.starts_with("shed_")
}

/// The floor for app events; library events always sit at WARN.
#[must_use]
fn filter_floor(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error | LogLevel::Critical => LevelFilter::ERROR,
    }
}

/// The filter decision as a pure function, the unit-test seam that does not
/// need a live subscriber.
#[must_use]
fn permits(setting: LogLevel, target: &str, event: Level) -> bool {
    let max = if is_app_target(target) {
        filter_floor(setting)
    } else {
        LevelFilter::WARN
    };
    event <= max
}

#[cfg(test)]
mod tests {
    use super::{permits, startup_line, Level, LogLevel};

    #[test]
    fn startup_line_is_python_wording() {
        assert_eq!(
            startup_line(LogLevel::Info),
            "Logging at INFO (set LOG_LEVEL to change); app lines only"
        );
        assert_eq!(
            startup_line(LogLevel::Warn),
            "Logging at WARNING (set LOG_LEVEL to change); app lines only"
        );
    }

    #[test]
    fn app_events_follow_the_setting() {
        assert!(permits(LogLevel::Info, "shed_chat::turn", Level::INFO));
        assert!(!permits(LogLevel::Info, "shed_chat::turn", Level::DEBUG));
        assert!(permits(LogLevel::Debug, "shed_chat::turn", Level::DEBUG));
        assert!(!permits(LogLevel::Error, "shed_chat::turn", Level::INFO));
        assert!(permits(LogLevel::Critical, "shed_chat::turn", Level::ERROR));
    }

    /// The propagate=False mirror: library records only reach the log at
    /// WARN no matter how chatty the app setting is.
    #[test]
    fn library_events_are_pinned_at_warn() {
        assert!(!permits(LogLevel::Debug, "hyper::proto", Level::INFO));
        assert!(!permits(LogLevel::Debug, "tower_http::trace", Level::DEBUG));
        assert!(permits(LogLevel::Debug, "hyper::client", Level::WARN));
        assert!(permits(LogLevel::Critical, "hyper::client", Level::ERROR));
    }
}
