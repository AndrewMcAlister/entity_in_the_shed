//! Where the app keeps its runtime state.
//!
//! Ported from `utils/env.py:10` (`resolve_storage_dir`) and `agent.py:305`.

use std::path::{Path, PathBuf};

use crate::ConfigError;
use crate::env::Env;

/// `resolve_storage_dir`: a configured directory, made absolute against `base_dir` when it is
/// not, and created if it does not exist.
///
/// An empty or blank setting falls back to `default_path` rather than producing a relative
/// path of `""`, which is what the Python guard does before it joins anything.
pub fn resolve_storage_dir(
    setting_value: &str,
    default_path: &Path,
    base_dir: &Path,
) -> Result<PathBuf, ConfigError> {
    let setting = setting_value.trim();
    let mut candidate = if setting.is_empty() {
        default_path.to_path_buf()
    } else {
        PathBuf::from(setting)
    };

    if !candidate.is_absolute() {
        candidate = base_dir.join(candidate);
    }

    std::fs::create_dir_all(&candidate)
        .map_err(|source| ConfigError::Storage { path: candidate.display().to_string(), source })?;
    Ok(candidate)
}

/// Where runtime state lives when `STORAGE_ROOT` is not set.
///
/// The Python default is `C:\Temp\entity-in-the-shed`. The clone moves it under the account's
/// local application data instead, which is `How_To_Code` principle 19 in its literal form:
/// transcripts, mind state and generated audio are runtime state, and a temp directory is
/// exactly where an unrelated cleanup sweeps them away.
pub fn default_storage_root(env: &Env) -> PathBuf {
    // The two names are written at the read site rather than looped over, so every setting this
    // crate reads appears as a literal in the code. That is what lets the `.env.example` gate in
    // `shed-gates` list the names by reading the crate instead of guessing at them.
    for value in [env.get("LOCALAPPDATA"), env.get("HOME")] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            return PathBuf::from(value.trim()).join("entity-in-the-shed");
        }
    }
    PathBuf::from("entity-in-the-shed")
}

/// The storage root, resolved and created.
///
/// The sub-directories the app writes into (`Conversations/`, `Mind/`, `Logs/`, `tts-cache/`)
/// are added to this struct by the stages that write into them, so no directory is created
/// here before something exists that will put a file in it.
#[derive(Debug, Clone)]
pub struct Storage {
    pub root: PathBuf,
}

impl Storage {
    /// `STORAGE_ROOT`, resolved against `base_dir` and created.
    pub fn resolve(env: &Env, base_dir: &Path) -> Result<Self, ConfigError> {
        let setting = env.text("STORAGE_ROOT", "");
        let root = resolve_storage_dir(&setting, &default_storage_root(env), base_dir)?;
        Ok(Self { root })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Each test gets its own directory under the approved temporary tree, the way
    /// `tempfile.TemporaryDirectory()` does in the ported suite.
    fn scratch(label: &str) -> Result<PathBuf, ConfigError> {
        let dir = std::env::temp_dir().join("shed-config-storage-tests").join(label);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .map_err(|source| ConfigError::Storage { path: dir.display().to_string(), source })?;
        Ok(dir)
    }

    #[test]
    fn absolute_setting_is_used_as_given_and_created() -> Result<(), ConfigError> {
        let base = scratch("absolute")?;
        let target = base.join("sub");
        let resolved =
            resolve_storage_dir(&target.display().to_string(), Path::new("unused"), &base)?;
        assert_eq!(resolved, target);
        assert!(target.is_dir());
        Ok(())
    }

    #[test]
    fn relative_setting_resolves_against_the_base_dir() -> Result<(), ConfigError> {
        let base = scratch("relative")?;
        let resolved = resolve_storage_dir("nested/deep", Path::new("unused"), &base)?;
        assert_eq!(resolved, base.join("nested").join("deep"));
        assert!(resolved.is_dir());
        Ok(())
    }

    #[test]
    fn blank_setting_falls_back_to_the_default_path() -> Result<(), ConfigError> {
        let base = scratch("blank")?;
        let default_path = base.join("default_dir");
        for setting in ["", "   "] {
            let resolved = resolve_storage_dir(setting, &default_path, &base)?;
            assert_eq!(resolved, default_path);
            assert!(default_path.is_dir());
        }
        Ok(())
    }

    #[test]
    fn the_default_root_sits_under_the_account_profile_not_the_repo() {
        let env = Env::with_values([("LOCALAPPDATA".to_string(), String::from("R:\\AppData"))]);
        let root = default_storage_root(&env);
        assert_eq!(root, PathBuf::from("R:\\AppData").join("entity-in-the-shed"));

        // No profile variable at all still names a directory outside the source tree.
        assert_eq!(default_storage_root(&Env::default()), PathBuf::from("entity-in-the-shed"));
    }

    #[test]
    fn storage_reads_the_setting_verbatim_and_creates_it() -> Result<(), ConfigError> {
        let base = scratch("setting")?;
        let env = Env::with_values([(
            "STORAGE_ROOT".to_string(),
            base.join("runtime").display().to_string(),
        )]);
        let storage = Storage::resolve(&env, &base)?;
        assert_eq!(storage.root, base.join("runtime"));
        assert!(storage.root.is_dir());
        Ok(())
    }

    #[test]
    fn a_directory_that_cannot_be_created_names_the_path_it_tried() -> Result<(), ConfigError> {
        let base = scratch("blocked")?;
        let blocker = base.join("not-a-dir");
        assert!(fs::write(&blocker, "a file in the way").is_ok(), "{blocker:?}");

        let error = resolve_storage_dir(
            &blocker.join("child").display().to_string(),
            Path::new("unused"),
            &base,
        )
        .err();

        assert!(
            matches!(&error, Some(ConfigError::Storage { path, .. }) if path.contains("not-a-dir")),
            "{error:?}"
        );
        Ok(())
    }
}
