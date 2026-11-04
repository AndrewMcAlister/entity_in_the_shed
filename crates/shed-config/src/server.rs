//! The address the HTTP server binds.
//!
//! Ported from `agent.py:574-575` (`APP_HOST`, `APP_PORT`).
//!
//! The source spells these `FLASK_HOST` with a `HOST` fallback and `FLASK_PORT` with a `PORT`
//! fallback. The clone uses one spelling each, named for the thing they address rather than for
//! the framework the source ran on (`PLAN-Rust-Clone.md` §5). The values and the defaults are
//! unchanged, so `0.0.0.0:5005` is still the address the app answers on when nothing is set.

use crate::ConfigError;
use crate::env::Env;

/// `APP_HOST`'s fallback: every interface, which is what the source used.
pub const DEFAULT_HOST: &str = "0.0.0.0";

/// `APP_PORT`'s fallback, and the port the operator's own settings name.
pub const DEFAULT_PORT: u16 = 5005;

#[derive(Debug, Clone)]
pub struct Server {
    pub host: String,
    pub port: u16,
}

impl Server {
    pub fn read(env: &Env) -> Result<Self, ConfigError> {
        // `os.getenv` keeps an empty value, so `APP_HOST=` is a host of `""` rather than an
        // absent setting, and the source binds that to every interface. The empty string is
        // carried through here so the configuration says what it says; `bind_host` is where it
        // becomes an address.
        let host = env.text("APP_HOST", DEFAULT_HOST);

        // `int(os.getenv(...))` raises at import in Python, so a value that is not a number is
        // an error here rather than a quiet fallback to 5005: a server that starts on the wrong
        // port answers on an address nothing is looking at.
        let raw = env.text("APP_PORT", &DEFAULT_PORT.to_string());
        let port =
            raw.trim().parse::<u16>().map_err(|_| ConfigError::invalid_port("APP_PORT", &raw))?;

        Ok(Self { host, port })
    }

    /// The host to hand the socket.
    ///
    /// An empty `APP_HOST` binds every interface, because `socket.bind(("", port))` is
    /// `INADDR_ANY`, which is what the source's `app.run(host="")` does.
    pub fn bind_host(&self) -> &str {
        if self.host.is_empty() { DEFAULT_HOST } else { &self.host }
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

    #[test]
    fn the_two_settings_are_read_as_given_and_the_defaults_fill_an_absence()
    -> Result<(), ConfigError> {
        let set = Server::read(&env_with(&[("APP_HOST", "127.0.0.1"), ("APP_PORT", "8080")]))?;
        assert_eq!(set.host, "127.0.0.1");
        assert_eq!(set.port, 8080);

        let nothing = Server::read(&Env::default())?;
        assert_eq!(nothing.host, DEFAULT_HOST);
        assert_eq!(nothing.port, DEFAULT_PORT);
        Ok(())
    }

    #[test]
    fn the_source_spellings_are_not_read_because_the_clone_names_the_settings_once() {
        // `FLASK_HOST`/`HOST`/`FLASK_PORT`/`PORT` are the source's four names (§5). Reading them
        // would mean two spellings for one setting, and a value in a `.env` the clone never
        // wrote would silently decide which one won.
        let legacy = Server::read(&env_with(&[
            ("FLASK_HOST", "10.0.0.1"),
            ("HOST", "10.0.0.2"),
            ("FLASK_PORT", "5006"),
            ("PORT", "5007"),
        ]))
        .ok();
        assert_eq!(
            legacy.map(|server| (server.host, server.port)),
            Some((String::from("0.0.0.0"), DEFAULT_PORT))
        );
    }

    #[test]
    fn a_blank_host_is_a_value_that_binds_every_interface() {
        let server = Server::read(&env_with(&[("APP_HOST", "")])).ok();
        assert_eq!(server.as_ref().map(|server| server.host.as_str()), Some(""));
        assert_eq!(server.as_ref().map(|server| server.bind_host()), Some(DEFAULT_HOST));
    }

    #[test]
    fn a_port_that_is_not_a_number_is_an_error_that_names_the_setting() {
        for value in ["abc", "5005.5", "", "70000"] {
            let error = Server::read(&env_with(&[("APP_PORT", value)])).err();
            assert!(
                matches!(&error, Some(ConfigError::InvalidPort { name, .. }) if name == "APP_PORT"),
                "{value:?} must be rejected: {error:?}"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_the_port() -> Result<(), ConfigError> {
        assert_eq!(Server::read(&env_with(&[("APP_PORT", " 5005 ")]))?.port, 5005);
        Ok(())
    }
}
