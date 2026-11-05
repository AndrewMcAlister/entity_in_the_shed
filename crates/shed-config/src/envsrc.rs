//! The only module in the workspace that touches the process environment
//! (principle 19; the `shed-gates::env_read` gate enforces the boundary).
//!
//! Merge order mirrors Python exactly: `load_dotenv(BASE_DIR/.env,
//! override=True)` at `agent.py:299` wrote file values over the process
//! environment before any read happened, so a `.env` entry always wins.
//! Python then kept mutating `os.environ` because 148 scattered reads needed
//! it; the clone hands every crate the typed [`crate::AppConfig`] instead, so
//! the merged view never has to be written back — process state stays
//! untouched and tests inject values without global mutation.

use std::collections::BTreeMap;
use std::path::Path;

use crate::envfile;
use crate::ConfigError;

/// A read-only view of `.env` overlaid on the process environment.
#[derive(Debug, Clone, Default)]
pub struct EnvSource {
    file: BTreeMap<String, String>,
}

impl EnvSource {
    /// Read and parse `path`. A missing file is fine (the app runs on pure
    /// process environment then); an unreadable one is a startup failure —
    /// python-dotenv swallowed that error silently and booted on defaults,
    /// which is how a half-written `.env` becomes a surprise at 3 a.m.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ConfigError(format!(
                    "cannot read {}: {error}",
                    path.display()
                )))
            }
        };
        Ok(Self {
            file: envfile::parse(&text, &process_lookup),
        })
    }

    /// Test seam: entries exactly as a `.env` file would provide them.
    #[must_use]
    pub fn from_file_values(file: BTreeMap<String, String>) -> Self {
        Self { file }
    }

    /// `.env` value first (the `override=True` semantics), process
    /// environment second.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        match self.file.get(name) {
            Some(value) => Some(value.clone()),
            None => process_lookup(name),
        }
    }
}

/// The workspace's single process-environment read. Dynamic on purpose:
/// interpolation and the generic `get` take computed names, and the
/// settings gate only counts literal names at `config` call sites.
fn process_lookup(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[cfg(test)]
mod tests {
    use super::EnvSource;
    use std::collections::BTreeMap;

    #[test]
    fn file_values_win_over_the_process_environment() {
        // PATH exists in every real process environment; a file entry named
        // PATH must shadow it, which is the override=True contract.
        let mut file = BTreeMap::new();
        file.insert("PATH".to_owned(), "from-file".to_owned());
        let source = EnvSource::from_file_values(file);
        assert_eq!(source.get("PATH").as_deref(), Some("from-file"));
    }

    #[test]
    fn missing_name_is_none_and_process_fallback_works() {
        let source = EnvSource::default();
        assert_eq!(source.get("SHED_CONFIG_TEST_ABSENT_NAME"), None);
        assert!(source.get("PATH").is_some(), "PATH is set in every shell");
    }

    #[test]
    fn loading_a_missing_file_is_fine() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!(
            "shed-config-envsrc-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        let source = EnvSource::load(&dir.join("absent.env"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(source?.get("SHED_CONFIG_TEST_ABSENT_NAME").is_none());
        Ok(())
    }
}
