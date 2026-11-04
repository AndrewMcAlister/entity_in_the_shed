//! Typed `AppConfig`: every environment name is read once, validated and stored here. No
//! other crate reads the environment (`How_To_Code` principles 19 and 24).
//!
//! The Python app keeps its settings in module-level globals that every function in
//! `agent.py` can reach. Here the settings are grouped into the sub-struct a domain owns, and
//! a sub-struct exists only once a stage consumes it: `shed-config` carries the storage root,
//! the bind address, the two model names, the McTrader MCP settings and the log level, because
//! those are what Stage 1 boots. The rest of the 113 documented settings arrive with the
//! routes that read them, which is also why `.env.example` grows stage by stage rather than
//! listing everything at once.
//!
//! ## Environment names
//!
//! | Name | Stage-1 consumer |
//! |---|---|
//! | `STORAGE_ROOT` | the storage root, created at startup |
//! | `APP_HOST` | the bind address |
//! | `APP_PORT` | the bind port |
//! | `AI_MODEL`, `LM_ENGINE_NORMAL_MODEL` | `GET /health` |
//! | `MCTRADER_MCP_TRANSPORT`, `_COMMAND`, `_ARGS`, `_ENV`, `_URL`, `_TIMEOUT`, `_SERVER_NAME` | the `mctrader_mcp` block of `GET /health` |
//! | `LOG_LEVEL` | which lines the app writes |

use std::io;
use std::path::Path;

use thiserror::Error;

mod dotenv;
mod env;
mod json_env;
mod logging_cfg;
mod mcp;
mod models;
mod server;
mod storage;

pub use env::Env;
pub use logging_cfg::LogLevel;
pub use mcp::{McpConfig, Transport};
pub use models::ModelNames;
pub use server::{DEFAULT_HOST, DEFAULT_PORT, Server};
pub use storage::Storage;

/// A setting that was present but could not be used.
///
/// Every variant names the environment variable, because the person fixing it is reading a
/// `.env` file and needs to know which line. The JSON messages keep the Python wording
/// (`{NAME} must be valid JSON: ...`, `{NAME} must decode to a ...`) so a captured error
/// message from the Python app still reads the same way here.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{name} must be valid JSON: {message}")]
    InvalidJson { name: String, message: String },

    #[error("{name} must decode to a {expected}")]
    WrongKind { name: String, expected: &'static str },

    #[error("{name} must be a port number, got {value}")]
    InvalidPort { name: String, value: String },

    #[error("storage root {path} could not be created: {source}")]
    Storage {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("the working directory could not be read: {0}")]
    NoWorkingDir(#[source] io::Error),
}

impl ConfigError {
    pub(crate) fn invalid_json(name: &str, message: String) -> Self {
        Self::InvalidJson { name: name.to_string(), message }
    }

    pub(crate) fn wrong_kind(name: &str, expected: &'static str) -> Self {
        Self::WrongKind { name: name.to_string(), expected }
    }

    pub(crate) fn invalid_port(name: &str, value: &str) -> Self {
        Self::InvalidPort { name: name.to_string(), value: value.to_string() }
    }
}

/// Everything this stage of the app needs to know about the machine it runs on.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub storage: Storage,
    pub server: Server,
    pub models: ModelNames,
    pub mcp: McpConfig,
    pub log_level: LogLevel,
}

impl AppConfig {
    /// Read the process environment, let `.env` override it, and build the configuration.
    ///
    /// `base_dir` is the working directory, which is the clone's `BASE_DIR`: `agent.py` resolves
    /// a relative `STORAGE_ROOT` against the folder that holds it, and a binary has no such
    /// folder. The Stage 8 service installer sets the working directory for the same reason.
    pub fn load() -> Result<Self, ConfigError> {
        let base_dir = std::env::current_dir().map_err(ConfigError::NoWorkingDir)?;
        let env = Env::from_process().with_dotenv_overrides(&base_dir.join(".env"));
        Self::load_from(&env, &base_dir)
    }

    /// The same build from a table of values and a base directory, which is what the tests use.
    pub fn load_from(env: &Env, base_dir: &Path) -> Result<Self, ConfigError> {
        Ok(Self {
            storage: Storage::resolve(env, base_dir)?,
            server: Server::read(env)?,
            models: ModelNames::read(env),
            mcp: McpConfig::read(env)?,
            log_level: LogLevel::from_setting(env),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A directory of its own under the approved temporary tree, the way
    /// `tempfile.TemporaryDirectory()` is used in the ported suite.
    fn scratch(label: &str) -> Result<PathBuf, ConfigError> {
        let dir = std::env::temp_dir().join("shed-config-app-tests").join(label);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|source| ConfigError::Storage { path: dir.display().to_string(), source })?;
        Ok(dir)
    }

    fn write_file(base: &Path, name: &str, text: &str) -> Result<(), ConfigError> {
        let path = base.join(name);
        std::fs::write(&path, text)
            .map_err(|source| ConfigError::Storage { path: path.display().to_string(), source })
    }

    #[test]
    fn a_bare_environment_produces_the_defaults() -> Result<(), ConfigError> {
        let base = scratch("defaults")?;
        let config = AppConfig::load_from(&Env::default(), &base)?;

        assert_eq!(config.storage.root, base.join("entity-in-the-shed"));
        assert!(config.storage.root.is_dir());
        assert_eq!(config.server.host, DEFAULT_HOST);
        assert_eq!(config.server.port, DEFAULT_PORT);
        assert_eq!(config.models.ai_model, "");
        assert_eq!(config.models.normal_model, "");
        assert!(!config.mcp.is_configured());
        assert_eq!(config.log_level, LogLevel::Info);
        Ok(())
    }

    #[test]
    fn a_dotenv_file_overrides_a_value_the_launcher_set() -> Result<(), ConfigError> {
        let base = scratch("dotenv")?;
        write_file(&base, ".env", "APP_PORT=5010\nLOG_LEVEL=warning\n")?;

        let launcher = Env::with_values([
            ("APP_PORT".to_string(), String::from("5005")),
            ("LOG_LEVEL".to_string(), String::from("debug")),
        ]);
        let with_file = launcher.with_dotenv_overrides(&base.join(".env"));

        // `load_dotenv(..., override=True)`: the file wins.
        assert_eq!(with_file.text("APP_PORT", ""), "5010");
        assert_eq!(with_file.text("LOG_LEVEL", ""), "warning");

        let config = AppConfig::load_from(&with_file, &base)?;
        assert_eq!(config.server.port, 5010);
        assert_eq!(config.log_level, LogLevel::Warn);
        Ok(())
    }

    #[test]
    fn a_missing_dotenv_is_not_an_error() -> Result<(), ConfigError> {
        let base = scratch("no-dotenv")?;
        let env = Env::with_values([("APP_PORT".to_string(), String::from("5005"))])
            .with_dotenv_overrides(&base.join("absent.env"));
        assert_eq!(env.text("APP_PORT", ""), "5005");
        Ok(())
    }

    #[test]
    fn a_bad_setting_names_the_one_that_is_wrong() -> Result<(), ConfigError> {
        let base = scratch("bad-setting")?;
        let env = Env::with_values([
            ("MCTRADER_MCP_COMMAND".to_string(), String::from("cmd.exe")),
            ("MCTRADER_MCP_ARGS".to_string(), String::from("not json")),
        ]);
        let error = AppConfig::load_from(&env, &base).err();
        assert!(
            matches!(&error, Some(ConfigError::InvalidJson { name, .. }) if name == "MCTRADER_MCP_ARGS"),
            "{error:?}"
        );
        Ok(())
    }
}
