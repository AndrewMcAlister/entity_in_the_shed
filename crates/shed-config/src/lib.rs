//! Typed application configuration (How_To_Code principle 19).
//!
//! The Python app read the environment in 148 scattered places; here one
//! [`AppConfig`] parses every name once, validates it, and hands the rest of
//! the workspace an immutable struct. This is the only crate allowed to read
//! the process environment — `shed-gates::env_read` enforces that, and
//! [`envsrc`] is the single module holding the read calls.
//!
//! Startup validation follows principle 14: an invalid value is a hard
//! failure that names the offending key, before any task starts.

pub mod config;
pub mod envfile;
pub mod envsrc;
pub mod parse;

pub use config::{AppConfig, LogLevel};
pub use envsrc::EnvSource;

/// A startup configuration failure, always naming the key (or file) at fault.
/// Python had no such type — bad values crashed deep inside import-time code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigError {}
