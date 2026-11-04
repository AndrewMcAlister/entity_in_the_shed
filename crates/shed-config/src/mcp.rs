//! The McTrader MCP server settings: what is configured, and whether it is usable.
//!
//! Ported from `agent.py:470-475` (the reads), `agent.py:1764-1802`
//! (`resolve_mctrader_mcp_transport`, `build_mctrader_mcp_config`,
//! `validate_mctrader_mcp_config`) and `utils/mcp.py:14` (the server name).
//!
//! Stage 1 ports the settings and their validation, because `GET /health` reports them. What
//! Stage 1 does not do is connect: `tool_count` and `tools` stay empty until the MCP client
//! arrives in Stage 5.

use serde_json::{Map, Value};

use crate::ConfigError;
use crate::env::Env;
use crate::json_env::{parse_json_list, parse_json_object};

/// `MCTRADER_MCP_SERVER_NAME`'s fallback (`utils/mcp.py:14`). The name is what the MCP session
/// calls the server, and a blank setting is treated as no setting.
pub const DEFAULT_SERVER_NAME: &str = "mctrader";

/// The `MCTRADER_MCP_TIMEOUT` fallback (`agent.py:475`).
pub const DEFAULT_TIMEOUT_SECONDS: i64 = 600;

/// The floor a configured timeout is raised to. `agent.py` clamps twice — `max(1, …)` at the
/// read and `max(5, …)` when the config is built — and the wider floor is the one that binds.
pub const MINIMUM_TIMEOUT_SECONDS: i64 = 5;

/// What `get_mctrader_mcp_status` reports when nothing is configured (`agent.py:1797`). The
/// text is part of the `/health` payload, so it is a constant rather than a message a caller
/// could reword.
pub const UNCONFIGURED_MESSAGE: &str =
    "Set MCTRADER_MCP_COMMAND, MCTRADER_MCP_URL, or set MCTRADER_MCP_TRANSPORT explicitly.";

/// How the MCP server is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Sse,
    StreamableHttp,
}

impl Transport {
    /// The spelling the MCP layer and `/health` use for this transport.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
            Self::StreamableHttp => "streamable-http",
        }
    }

    /// The three spellings `resolve_mctrader_mcp_transport` accepts; anything else is not an
    /// explicit choice of transport.
    fn parse(text: &str) -> Option<Self> {
        match text {
            "stdio" => Some(Self::Stdio),
            "sse" => Some(Self::Sse),
            "streamable-http" => Some(Self::StreamableHttp),
            _ => None,
        }
    }

    /// Whether the server is reached over HTTP rather than as a child process.
    fn over_http(self) -> bool {
        matches!(self, Self::Sse | Self::StreamableHttp)
    }
}

/// The configured server, in the shape `build_mctrader_mcp_config` builds.
#[derive(Debug, Clone)]
pub struct McpConfig {
    pub server_name: String,
    /// `None` is the unconfigured state, and it is what `/health` reports as
    /// `transport: ""`.
    pub transport: Option<Transport>,
    pub timeout_seconds: i64,
    /// Meaningful for a stdio server only.
    pub command: String,
    /// Parsed from `MCTRADER_MCP_ARGS`, and only when the transport is stdio.
    pub args: Vec<Value>,
    /// Parsed from `MCTRADER_MCP_ENV`, and only when the transport is stdio.
    pub server_env: Map<String, Value>,
    /// Meaningful for an HTTP server only.
    pub url: String,
}

impl McpConfig {
    /// Read the six `MCTRADER_MCP_*` settings and the server name, exactly as `agent.py` reads
    /// them: the transport name is lower-cased, the rest are trimmed.
    pub fn read(env: &Env) -> Result<Self, ConfigError> {
        let transport_text = env.text("MCTRADER_MCP_TRANSPORT", "").trim().to_lowercase();
        let command = env.text("MCTRADER_MCP_COMMAND", "").trim().to_string();
        let url = env.text("MCTRADER_MCP_URL", "").trim().to_string();
        let server_name = {
            let name = env.text("MCTRADER_MCP_SERVER_NAME", "").trim().to_string();
            if name.is_empty() {
                DEFAULT_SERVER_NAME.to_string()
            } else {
                name
            }
        };

        let transport = Transport::parse(&transport_text).or_else(|| {
            // No usable explicit transport: infer it from what is set, which is how a
            // `.env` that only names a command still reaches the server.
            if !command.is_empty() {
                return Some(Transport::Stdio);
            }
            if !url.is_empty() {
                return Some(Transport::Sse);
            }
            None
        });

        // The JSON-valued settings are parsed only in the branch that uses them, so a malformed
        // MCTRADER_MCP_ARGS cannot break an HTTP configuration.
        let (args, server_env) = if transport == Some(Transport::Stdio) {
            (
                parse_json_list("MCTRADER_MCP_ARGS", env.get("MCTRADER_MCP_ARGS"))?,
                parse_json_object("MCTRADER_MCP_ENV", env.get("MCTRADER_MCP_ENV"))?,
            )
        } else {
            (Vec::new(), Map::new())
        };

        // Read on one line, rather than chained inside the struct literal: the settings gate in
        // `shed-gates` lists the names this crate reads by finding `env.<method>("NAME")` in the
        // shipped code, and a read that wraps across lines would not show up in that list.
        let timeout = env.int("MCTRADER_MCP_TIMEOUT", DEFAULT_TIMEOUT_SECONDS);

        Ok(Self {
            server_name,
            transport,
            timeout_seconds: timeout.max(MINIMUM_TIMEOUT_SECONDS),
            command,
            args,
            server_env,
            url,
        })
    }

    /// `validate_mctrader_mcp_config`: the transport has to be reachable, and the setting that
    /// reaches it has to be there.
    pub fn validation_error(&self) -> Option<String> {
        let Some(transport) = self.transport else {
            return Some(UNCONFIGURED_MESSAGE.to_string());
        };
        if transport == Transport::Stdio && self.command.trim().is_empty() {
            return Some("MCTRADER_MCP_COMMAND is not configured".to_string());
        }
        if transport.over_http() && self.url.trim().is_empty() {
            return Some("MCTRADER_MCP_URL is not configured".to_string());
        }
        None
    }

    /// Whether this configuration can open a session at all.
    pub fn is_configured(&self) -> bool {
        self.validation_error().is_none()
    }

    /// The transport as `/health` spells it: an empty string when there is none.
    pub fn transport_text(&self) -> String {
        self.transport.map_or_else(String::new, |t| t.as_str().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, &str)]) -> Env {
        Env::with_values(
            pairs.iter().map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        )
    }

    fn config(pairs: &[(&str, &str)]) -> Result<McpConfig, ConfigError> {
        McpConfig::read(&env_with(pairs))
    }

    /// The table captured off the running Python app, not inferred from its source text.
    #[test]
    fn the_transport_is_the_explicit_one_or_the_inferred_one() -> Result<(), ConfigError> {
        let cases = [
            (Vec::new(), None),
            (
                vec![("MCTRADER_MCP_TRANSPORT", "stdio"), ("MCTRADER_MCP_COMMAND", "cmd.exe")],
                Some(Transport::Stdio),
            ),
            (vec![("MCTRADER_MCP_COMMAND", "cmd.exe")], Some(Transport::Stdio)),
            (vec![("MCTRADER_MCP_URL", "http://127.0.0.1:8765/mcp")], Some(Transport::Sse)),
            (
                vec![
                    ("MCTRADER_MCP_TRANSPORT", "streamable-http"),
                    ("MCTRADER_MCP_URL", "http://127.0.0.1:8765/mcp"),
                ],
                Some(Transport::StreamableHttp),
            ),
            // A transport name that is not one of the three is not a choice: the app falls
            // back to inferring from the command or the URL.
            (
                vec![
                    ("MCTRADER_MCP_TRANSPORT", "bogus"),
                    ("MCTRADER_MCP_URL", "http://127.0.0.1:8765/mcp"),
                ],
                Some(Transport::Sse),
            ),
            (
                vec![("MCTRADER_MCP_TRANSPORT", "bogus"), ("MCTRADER_MCP_COMMAND", "cmd.exe")],
                Some(Transport::Stdio),
            ),
        ];

        for (pairs, expected) in cases {
            let parsed = config(&pairs)?;
            assert_eq!(parsed.transport, expected, "{pairs:?}");
        }
        Ok(())
    }

    #[test]
    fn nothing_configured_is_the_documented_state() -> Result<(), ConfigError> {
        let parsed = config(&[])?;
        assert_eq!(parsed.server_name, DEFAULT_SERVER_NAME);
        assert_eq!(parsed.transport, None);
        assert_eq!(parsed.transport_text(), "");
        assert_eq!(parsed.timeout_seconds, 600);
        assert!(parsed.command.is_empty());
        assert!(parsed.args.is_empty());
        assert!(parsed.server_env.is_empty());
        assert!(parsed.url.is_empty());
        assert!(!parsed.is_configured());
        assert_eq!(parsed.validation_error().as_deref(), Some(UNCONFIGURED_MESSAGE));
        Ok(())
    }

    #[test]
    fn a_transport_without_the_setting_that_reaches_it_is_invalid() {
        let stdio = config(&[("MCTRADER_MCP_TRANSPORT", "stdio")]).ok();
        assert_eq!(
            stdio.and_then(|parsed| parsed.validation_error()).as_deref(),
            Some("MCTRADER_MCP_COMMAND is not configured")
        );

        let http = config(&[("MCTRADER_MCP_TRANSPORT", "streamable-http")]).ok();
        assert_eq!(
            http.and_then(|parsed| parsed.validation_error()).as_deref(),
            Some("MCTRADER_MCP_URL is not configured")
        );
    }

    #[test]
    fn timeout_is_floored_and_never_negative() -> Result<(), ConfigError> {
        assert_eq!(config(&[("MCTRADER_MCP_TIMEOUT", "0")])?.timeout_seconds, 5);
        assert_eq!(config(&[("MCTRADER_MCP_TIMEOUT", "-20")])?.timeout_seconds, 5);
        assert_eq!(config(&[("MCTRADER_MCP_TIMEOUT", "45")])?.timeout_seconds, 45);
        assert_eq!(
            config(&[("MCTRADER_MCP_TIMEOUT", "abc")])?.timeout_seconds,
            DEFAULT_TIMEOUT_SECONDS
        );
        Ok(())
    }

    #[test]
    fn the_json_settings_are_read_only_for_a_stdio_server() -> Result<(), ConfigError> {
        let stdio = config(&[
            ("MCTRADER_MCP_COMMAND", "cmd.exe"),
            ("MCTRADER_MCP_ARGS", r#"["--serve", "--port", "8765"]"#),
            ("MCTRADER_MCP_ENV", r#"{"TOKEN": "t"}"#),
        ])?;
        assert_eq!(stdio.args.len(), 3);
        assert_eq!(stdio.server_env.get("TOKEN"), Some(&Value::from("t")));

        // The same malformed values are inert when the transport is HTTP, because nothing
        // reads them in that branch.
        let http = config(&[
            ("MCTRADER_MCP_URL", "http://127.0.0.1:8765/mcp"),
            ("MCTRADER_MCP_ARGS", "{not json"),
            ("MCTRADER_MCP_ENV", "also not"),
        ])?;
        assert!(http.args.is_empty());
        assert!(http.server_env.is_empty());
        assert!(http.is_configured());
        Ok(())
    }

    #[test]
    fn a_malformed_json_setting_fails_the_stdio_branch() {
        let error =
            config(&[("MCTRADER_MCP_COMMAND", "cmd.exe"), ("MCTRADER_MCP_ARGS", "{not json")])
                .err();
        assert!(
            matches!(&error, Some(ConfigError::InvalidJson { name, .. }) if name == "MCTRADER_MCP_ARGS"),
            "{error:?}"
        );
    }

    #[test]
    fn a_blank_server_name_is_no_name() -> Result<(), ConfigError> {
        let parsed = config(&[("MCTRADER_MCP_SERVER_NAME", "   ")])?;
        assert_eq!(parsed.server_name, DEFAULT_SERVER_NAME);
        let named = config(&[("MCTRADER_MCP_SERVER_NAME", " mctrader-beta ")])?;
        assert_eq!(named.server_name, "mctrader-beta");
        Ok(())
    }
}
