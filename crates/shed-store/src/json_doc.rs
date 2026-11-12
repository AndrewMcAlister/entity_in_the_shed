//! The document writer: `atomic_write_json`, and the `.bak` copy some writers ask for.
//!
//! Ported from `utils/entity_store.py:46-101`. The two writers that share a
//! document differ only in whether the previous version is kept:
//!
//! - a mind-state record keeps it, because a state file that cannot be read has
//!   to be recovered from something (`load_json_with_recovery`, Stage 7);
//! - a conversation transcript skips it (`agent.py:3919`), because it is
//!   rewritten every turn and a second copy of it beside each record is space
//!   spent preserving words that are already in the file they were copied from.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Whether the previous version of the file is copied to `.bak` first.
///
/// The source spells this `keep_backup` as a keyword argument; a named flag
/// rather than a positional `bool` is what keeps `atomic_write_json(path,
/// &record, Backup::Skip)` readable at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backup {
    /// Copy the file that is there now to `{name}.json.bak` before replacing it.
    Keep,
    /// Replace it and leave no copy.
    Skip,
}

/// A document that could not be stored.
///
/// Each variant names the step that failed and the paths it was working on,
/// which is what a log line needs to be actionable: "cannot write" tells a
/// person nothing, "cannot move `state.json.a1b2….tmp` onto `state.json`" tells
/// them the file was built and the move was refused, which on Windows means
/// something else has it open.
#[derive(Debug)]
pub enum WriteError {
    /// The directory the file lives in could not be made.
    CreateDir {
        dir: PathBuf,
        source: std::io::Error,
    },
    /// The value could not be rendered as JSON.
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// The temp file could not be created, filled or synced.
    Write {
        temp: PathBuf,
        source: std::io::Error,
    },
    /// The finished temp file could not be moved into place.
    Replace {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateDir { dir, source } => {
                write!(f, "cannot create {}: {source}", dir.display())
            }
            Self::Serialize { path, source } => {
                write!(f, "cannot serialise {} as JSON: {source}", path.display())
            }
            Self::Write { temp, source } => {
                write!(f, "cannot write {}: {source}", temp.display())
            }
            Self::Replace { from, to, source } => write!(
                f,
                "cannot move {} onto {}: {source}",
                from.display(),
                to.display()
            ),
        }
    }
}

impl std::error::Error for WriteError {}

/// The previous good version of `path`, kept beside it.
///
/// `Path(str(path) + ".bak")` verbatim: the suffix goes on the whole name, so a
/// record called `client-ab12.json` gets `client-ab12.json.bak`, not
/// `client-ab12.bak`.
#[must_use]
pub fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.to_string_lossy()))
}

/// Write `payload` as JSON so a reader sees either the old file or the new one.
///
/// The bytes are rendered *before* the temp file exists, as they are in the
/// source: a payload that cannot be serialised leaves no stray file behind.
///
/// The rendering is `json.dumps(payload, indent=2, ensure_ascii=False)` — two
/// spaces per level, non-ASCII written as itself. One difference is recorded in
/// `docs/PARITY.md`: the source opens its file in text mode, so the same
/// document is CRLF on Windows, while this writes LF everywhere. Nothing reads
/// these files across the two apps (decision D9 gives the clone its own storage
/// root) and JSON does not care, so the clone keeps the one shape.
///
/// # Errors
///
/// [`WriteError`] when the directory cannot be made, the payload cannot be
/// rendered, the temp file cannot be written, or the move into place is refused.
/// The temp file is removed in every one of those cases.
pub fn atomic_write_json(
    path: &Path,
    payload: &impl Serialize,
    backup: Backup,
) -> Result<(), WriteError> {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    std::fs::create_dir_all(parent).map_err(|source| WriteError::CreateDir {
        dir: parent.to_path_buf(),
        source,
    })?;

    let bytes = serde_json::to_vec_pretty(payload).map_err(|source| WriteError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;

    let temp = temp_path(path);
    write_then_replace(path, &temp, &bytes, backup).inspect_err(|_| {
        // The source's `except BaseException` arm: a temp file left behind by a
        // failed write would be picked up by nothing, but it is litter in a
        // directory the person reads, so it goes. The error itself passes
        // through untouched, which is why this is `inspect_err` and not `map_err`.
        let _ = std::fs::remove_file(&temp);
    })
}

/// Fill the temp file, copy the old document if asked to, then move it in.
fn write_then_replace(
    path: &Path,
    temp: &Path,
    bytes: &[u8],
    backup: Backup,
) -> Result<(), WriteError> {
    let mut file = std::fs::File::create(temp).map_err(|source| WriteError::Write {
        temp: temp.to_path_buf(),
        source,
    })?;
    // `os.fsync` is not a parameter here: every caller in the source leaves it
    // True, and a document that is not on disk before the rename is a document a
    // crash takes back.
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| WriteError::Write {
            temp: temp.to_path_buf(),
            source,
        })?;
    drop(file);

    if backup == Backup::Keep && path.exists() {
        // Losing the backup must never cost the caller the write, so a failure
        // here is reported and the replace proceeds. The source passes on it in
        // silence; a copy that cannot be made is worth one line in the log,
        // because it means the recovery path Stage 7 is about to lose.
        match std::fs::copy(path, backup_path(path)) {
            Ok(_) => {}
            Err(source) => {
                tracing::warn!("could not copy {} to its backup: {source}", path.display())
            }
        }
    }

    std::fs::rename(temp, path).map_err(|source| WriteError::Replace {
        from: temp.to_path_buf(),
        to: path.to_path_buf(),
        source,
    })
}

/// The temp file's name: the target's name, a random middle, `.tmp`, beside the
/// target — which is the whole reason the replace can be atomic.
///
/// The source uses `tempfile.mkstemp(dir=…, prefix=f"{name}.", suffix=".tmp")`;
/// the random middle is a uuid4 hex here, longer than mkstemp's eight characters
/// and never seen by anything but this function, since the file is renamed or
/// deleted before the call returns.
fn temp_path(path: &Path) -> PathBuf {
    let stem = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let random = uuid::Uuid::new_v4().simple().to_string();
    path.parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{stem}.{random}.tmp"))
}
