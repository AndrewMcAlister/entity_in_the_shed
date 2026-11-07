//! The typed application configuration.
//!
//! Stage 1 carries the slice the smoke-boot server needs: storage root, log
//! level, model names, the MCP config block `/health` reports, and the bind
//! address. Stage 2 adds the LM Studio engine block — its address, the
//! discovery timeout, the retry ladder and the models directory — because
//! that is what model discovery reads. Later stages extend this struct as
//! they port their config regions (`agent.py:376–955`); the `.env.example`
//! settings gate keeps the two in step.

use std::path::{Path, PathBuf};

use crate::envsrc::EnvSource;
use crate::lm::{LmEngine, LmStudio};
use crate::parse::{getenv_int, getenv_str, normalize_model_name, parse_json_list, parse_json_map};
use crate::ConfigError;

/// Clone default, deliberately distinct from the Python app's
/// `C:\Temp\entity-in-the-shed` so both services can run at once (plan D9).
pub const DEFAULT_STORAGE_ROOT: &str = r"C:\Temp\entity_in_the_shed";

/// The five levels Python's `logging` module exposes to `LOG_LEVEL`,
/// canonicalised: `WARN`/`FATAL` are the source's aliases, and everything
/// above ERROR shares the log crate's top filter level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

/// Everything the process knows about its configuration, parsed once.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Resolved and created at startup; never read again from the environment.
    pub storage_root: PathBuf,
    pub log_level: LogLevel,
    pub ai_model: String,
    pub lm_engine_normal_model: String,
    /// The engine Stage 2 discovers models from: address, timeout, retry ladder.
    pub lm: LmEngine,
    /// Where the model files live on disk — the root the disk scan walks.
    pub lm_studio: LmStudio,
    pub mcp: McpConfig,
    pub host: String,
    pub port: u16,
}

impl AppConfig {
    /// Boot-time load: `.env` next to the executable (the clone of
    /// `load_dotenv(BASE_DIR / ".env", override=True)`, `agent.py:299`),
    /// merged over the process environment, then validated in full
    /// (principle 14).
    pub fn load() -> Result<Self, ConfigError> {
        let base_dir = base_dir()?;
        let source = EnvSource::load(&base_dir.join(".env"))?;
        Self::from_source(&source, &base_dir)
    }

    /// Test seam: build from an injected source and base directory.
    pub fn from_source(src: &EnvSource, base_dir: &Path) -> Result<Self, ConfigError> {
        Ok(Self {
            storage_root: resolve_storage_dir(src, base_dir)?,
            log_level: LogLevel::from_setting(&getenv_str(src, "LOG_LEVEL", "INFO")),
            ai_model: normalize_model_name(&getenv_str(src, "AI_MODEL", ""), ""),
            lm_engine_normal_model: normalize_model_name(
                &getenv_str(src, "LM_ENGINE_NORMAL_MODEL", ""),
                "",
            ),
            lm: LmEngine::from_source(src),
            lm_studio: LmStudio::from_source(src),
            mcp: McpConfig::from_source(src)?,
            host: getenv_str(src, "APP_HOST", "0.0.0.0").trim().to_owned(),
            port: parse_port(src)?,
        })
    }
}

/// The McTrader MCP block (`agent.py:470–476`, `1764–1802`). Stage 1 parses
/// and validates it for `/health`; the live session arrives at Stage 5.
#[derive(Debug, Clone)]
pub struct McpConfig {
    pub server_name: String,
    transport_setting: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub url: String,
    /// Effective timeout: the raw `max(1, …)` floor raised to 5 seconds by
    /// `build_mctrader_mcp_config` (`agent.py:1780`).
    pub timeout_seconds: i64,
    pub cache_ttl_seconds: i64,
}

impl McpConfig {
    /// Parse the `MCTRADER_MCP_*` block from an environment source. Public so
    /// the HTTP layer's byte-parity tests can build synthetic configs.
    pub fn from_source(src: &EnvSource) -> Result<Self, ConfigError> {
        let server_name = getenv_str(src, "MCTRADER_MCP_SERVER_NAME", "mctrader")
            .trim()
            .to_owned();
        let server_name = if server_name.is_empty() {
            "mctrader".to_owned()
        } else {
            server_name
        };
        let timeout_raw = getenv_int(src, "MCTRADER_MCP_TIMEOUT", 600).max(1);
        Ok(Self {
            server_name,
            transport_setting: getenv_str(src, "MCTRADER_MCP_TRANSPORT", "")
                .trim()
                .to_lowercase(),
            command: getenv_str(src, "MCTRADER_MCP_COMMAND", "")
                .trim()
                .to_owned(),
            args: parse_json_list(
                "MCTRADER_MCP_ARGS",
                &getenv_str(src, "MCTRADER_MCP_ARGS", ""),
            )?,
            env: parse_json_map("MCTRADER_MCP_ENV", &getenv_str(src, "MCTRADER_MCP_ENV", ""))?,
            url: getenv_str(src, "MCTRADER_MCP_URL", "").trim().to_owned(),
            timeout_seconds: timeout_raw.max(5),
            cache_ttl_seconds: getenv_int(src, "MCTRADER_MCP_CACHE_TTL", 30).max(5),
        })
    }

    /// `resolve_mctrader_mcp_transport` (`agent.py:1764–1772`): an explicit
    /// valid transport wins; an invalid explicit value is ignored, then the
    /// command implies stdio and the url implies sse.
    #[must_use]
    pub fn transport(&self) -> &str {
        if matches!(
            self.transport_setting.as_str(),
            "stdio" | "sse" | "streamable-http"
        ) {
            return &self.transport_setting;
        }
        if !self.command.is_empty() {
            return "stdio";
        }
        if !self.url.is_empty() {
            return "sse";
        }
        ""
    }

    /// `validate_mctrader_mcp_config` (`agent.py:1793–1802`). The messages
    /// are contract text: `/health` surfaces them verbatim in `error`.
    pub fn validate(&self) -> Result<(), String> {
        let transport = self.transport();
        if transport.is_empty() {
            return Err("Set MCTRADER_MCP_COMMAND, MCTRADER_MCP_URL, or set MCTRADER_MCP_TRANSPORT explicitly.".to_owned());
        }
        if transport == "stdio" && self.command.trim().is_empty() {
            return Err("MCTRADER_MCP_COMMAND is not configured".to_owned());
        }
        if matches!(transport, "sse" | "streamable-http") && self.url.trim().is_empty() {
            return Err("MCTRADER_MCP_URL is not configured".to_owned());
        }
        Ok(())
    }

    /// The `error` half of the `/health` MCP block. Until the Stage 5 client
    /// exists there is no runtime cache error, so this is the validation
    /// message or empty — the Python cache error is a PARITY row.
    #[must_use]
    pub fn health_error(&self) -> String {
        match self.validate() {
            Ok(()) => String::new(),
            Err(message) => message,
        }
    }
}

/// `resolve_storage_dir` (`utils/env.py:10–15`): blank falls back to the
/// default; relative values resolve against the base directory; the result
/// is created before anything else boots.
fn resolve_storage_dir(src: &EnvSource, base_dir: &Path) -> Result<PathBuf, ConfigError> {
    let setting = getenv_str(src, "STORAGE_ROOT", "").trim().to_owned();
    let candidate = if setting.is_empty() {
        PathBuf::from(DEFAULT_STORAGE_ROOT)
    } else if Path::new(&setting).is_absolute() {
        PathBuf::from(&setting)
    } else {
        base_dir.join(&setting)
    };
    std::fs::create_dir_all(&candidate).map_err(|error| {
        ConfigError(format!(
            "STORAGE_ROOT cannot be created at {}: {error}",
            candidate.display()
        ))
    })?;
    Ok(candidate)
}

/// Python crashed on a bad `PORT` with an `int()` traceback during import;
/// the clone fails with a message naming the key — same fail-fast, readable.
fn parse_port(src: &EnvSource) -> Result<u16, ConfigError> {
    let raw = getenv_str(src, "APP_PORT", "5005").trim().to_owned();
    raw.parse::<u16>().map_err(|_| {
        ConfigError(format!(
            "APP_PORT must be an integer between 1 and 65535: '{raw}'"
        ))
    })
}

/// The directory the process treats as home: the executable's folder, where
/// the Stage 8 installer places the exe and its `.env`.
fn base_dir() -> Result<PathBuf, ConfigError> {
    let exe = std::env::current_exe()
        .map_err(|error| ConfigError(format!("cannot locate the running executable: {error}")))?;
    match exe.parent() {
        Some(dir) => Ok(dir.to_path_buf()),
        None => Err(ConfigError(
            "executable path has no parent directory".to_owned(),
        )),
    }
}

impl LogLevel {
    /// Python's `configure_logging` level lookup: case-insensitive level
    /// names (including the `WARN`/`FATAL` aliases and `CRITICAL`), anything
    /// else falls back to INFO.
    #[must_use]
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "debug" => Self::Debug,
            "warning" | "warn" => Self::Warn,
            "error" => Self::Error,
            "critical" | "fatal" => Self::Critical,
            "info" => Self::Info,
            _ => Self::Info,
        }
    }

    /// The name the Python startup line prints (`logging.getLevelName`).
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARNING",
            Self::Error => "ERROR",
            Self::Critical => "CRITICAL",
        }
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
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    fn mcp(pairs: &[(&str, &str)]) -> McpConfig {
        match McpConfig::from_source(&src(pairs)) {
            Ok(config) => config,
            Err(error) => panic!("unexpected MCP parse failure: {error}"),
        }
    }

    #[test]
    fn transport_resolution_matrix() {
        let explicit = mcp(&[("MCTRADER_MCP_TRANSPORT", "streamable-http")]);
        assert_eq!(explicit.transport(), "streamable-http");
        // An invalid explicit value is ignored; the command still implies stdio.
        let ignored = mcp(&[
            ("MCTRADER_MCP_TRANSPORT", "carrier-pigeon"),
            ("MCTRADER_MCP_COMMAND", "x"),
        ]);
        assert_eq!(ignored.transport(), "stdio");
        assert_eq!(mcp(&[("MCTRADER_MCP_COMMAND", "x")]).transport(), "stdio");
        assert_eq!(mcp(&[("MCTRADER_MCP_URL", "http://h")]).transport(), "sse");
        assert_eq!(mcp(&[]).transport(), "");
    }

    #[test]
    fn validation_messages_are_contract_text() {
        assert_eq!(
            mcp(&[]).validate().err(),
            Some("Set MCTRADER_MCP_COMMAND, MCTRADER_MCP_URL, or set MCTRADER_MCP_TRANSPORT explicitly.".to_owned())
        );
        assert_eq!(
            mcp(&[("MCTRADER_MCP_TRANSPORT", "stdio")]).validate().err(),
            Some("MCTRADER_MCP_COMMAND is not configured".to_owned())
        );
        assert_eq!(
            mcp(&[("MCTRADER_MCP_TRANSPORT", "streamable-http")])
                .validate()
                .err(),
            Some("MCTRADER_MCP_URL is not configured".to_owned())
        );
        let good = mcp(&[
            ("MCTRADER_MCP_TRANSPORT", "streamable-http"),
            ("MCTRADER_MCP_URL", "http://h"),
        ]);
        assert_eq!(good.validate(), Ok(()));
        assert_eq!(good.health_error(), "");
    }

    #[test]
    fn mcp_defaults_and_floors() -> Result<(), ConfigError> {
        let config = mcp(&[
            ("MCTRADER_MCP_SERVER_NAME", "  "),
            ("MCTRADER_MCP_TIMEOUT", "2"),
            ("MCTRADER_MCP_CACHE_TTL", "1"),
            ("MCTRADER_MCP_ARGS", r#"["a","b"]"#),
            ("MCTRADER_MCP_ENV", r#"{"K":"V"}"#),
        ]);
        assert_eq!(config.server_name, "mctrader", "blank name falls back");
        assert_eq!(config.timeout_seconds, 5, "build-config floor is 5s");
        assert_eq!(config.cache_ttl_seconds, 5);
        assert_eq!(config.args, vec!["a", "b"]);
        assert_eq!(config.env.get("K").map(String::as_str), Some("V"));
        Ok(())
    }

    #[test]
    fn bad_json_env_values_fail_naming_the_key() {
        let error = match McpConfig::from_source(&src(&[("MCTRADER_MCP_ARGS", "not-json")])) {
            Ok(_) => panic!("bad ARGS must fail the load"),
            Err(error) => error,
        };
        assert!(error.0.contains("MCTRADER_MCP_ARGS"), "{error}");
    }

    #[test]
    fn log_level_setting_mapping() {
        assert_eq!(LogLevel::from_setting("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from_setting(" WARNING "), LogLevel::Warn);
        assert_eq!(LogLevel::from_setting("warn"), LogLevel::Warn);
        assert_eq!(LogLevel::from_setting("Critical"), LogLevel::Critical);
        assert_eq!(LogLevel::from_setting("nonsense"), LogLevel::Info);
        assert_eq!(LogLevel::Critical.label(), "CRITICAL");
        assert_eq!(LogLevel::Warn.label(), "WARNING");
    }

    #[test]
    fn full_load_reads_every_configured_key() -> Result<(), ConfigError> {
        let root = std::env::temp_dir().join(format!("shed-config-load-{}", std::process::id()));
        let root_text = root.to_string_lossy().into_owned();
        let pairs = [
            ("STORAGE_ROOT", root_text.as_str()),
            ("LOG_LEVEL", "warning"),
            ("AI_MODEL", " model:latest "),
            ("LM_ENGINE_NORMAL_MODEL", "registry:5000/m:latest"),
            ("LM_ENGINE_HOST", "http://127.0.0.1:8080/"),
            ("LMSTUDIO_MODELS_DIR", r" D:\Models "),
            ("MCTRADER_MCP_URL", "http://127.0.0.1:8100/mcp"),
            ("MCTRADER_MCP_TRANSPORT", "streamable-http"),
            ("APP_HOST", " 127.0.0.1 "),
            ("APP_PORT", "5199"),
        ];
        let config = AppConfig::from_source(&src(&pairs), Path::new("."))?;
        assert_eq!(config.storage_root, root);
        assert!(root.is_dir(), "storage root is created at load");
        assert_eq!(config.log_level, LogLevel::Warn);
        assert_eq!(config.ai_model, "model:latest");
        assert_eq!(config.lm_engine_normal_model, "registry:5000/m");
        assert_eq!(config.lm.host, "http://127.0.0.1:8080");
        assert_eq!(
            config.lm_studio.models_dir,
            Some(std::path::PathBuf::from(r"D:\Models"))
        );
        assert!(config.mcp.validate().is_ok());
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 5199);
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn relative_storage_resolves_under_base_dir() -> Result<(), ConfigError> {
        let base = std::env::temp_dir().join(format!("shed-config-rel-{}", std::process::id()));
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        pairs.insert("STORAGE_ROOT".to_owned(), "nested/deep".to_owned());
        let config = AppConfig::from_source(&EnvSource::from_file_values(pairs), &base)?;
        assert_eq!(config.storage_root, base.join("nested").join("deep"));
        assert!(config.storage_root.is_dir());
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    #[test]
    fn bad_port_names_the_key() {
        let root = std::env::temp_dir().join(format!("shed-config-port-{}", std::process::id()));
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        pairs.insert(
            "STORAGE_ROOT".to_owned(),
            root.to_string_lossy().into_owned(),
        );
        pairs.insert("APP_PORT".to_owned(), "not-a-port".to_owned());
        let error = match AppConfig::from_source(
            &EnvSource::from_file_values(pairs),
            &std::env::temp_dir(),
        ) {
            Ok(_) => panic!("a bad port must fail the load"),
            Err(error) => error,
        };
        let _ = std::fs::remove_dir_all(&root);
        assert!(error.0.contains("APP_PORT"), "{error}");
    }
}
