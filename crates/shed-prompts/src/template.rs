//! The mode templates: their frontmatter, their paths, and the mtime cache
//! that loads them.
//!
//! `parse_prompt_markdown` and `_prompt_allows_laugh` are `utils/prompt.py:16`
//! and `:115`; `PROMPT_TEMPLATE_PATHS` and `load_prompt_template` are
//! `agent.py:600` and `:2106`, including that function's `PROMPT_TEMPLATE_CACHE`
//! (`agent.py:2136`) and the two `RuntimeError` texts it raises. The errors
//! travel as `String`s holding the exact message the Python exception carried.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use shed_text::py_splitlines;

/// One loaded template: its frontmatter and its body
/// (`agent.py:2131`'s `{"metadata": …, "body": …}` dict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// The `key: value` lines between the `---` fences, quotes peeled.
    pub metadata: BTreeMap<String, String>,
    /// Everything after the closing fence, trimmed.
    pub body: String,
}

/// `parse_prompt_markdown` (`utils/prompt.py:16`): the frontmatter fields and
/// the body of a `---`-fenced template.
///
/// A value is trimmed, then stripped of `"` and then of `'` — in that order,
/// so `"it's"` keeps its apostrophe and `'say "hi"'` keeps its quotes.
#[must_use]
pub fn parse_prompt_markdown(text: &str) -> (BTreeMap<String, String>, String) {
    let mut metadata = BTreeMap::new();
    let mut body = text;

    if text.starts_with("---") {
        // `content.split("---", 2)`: at most two splits, the remainder kept
        // whole. Fewer than three parts — one fence or none — is no
        // frontmatter at all.
        let parts: Vec<&str> = text.splitn(3, "---").collect();
        if parts.len() >= 3 {
            body = parts[2];
            for line in py_splitlines(parts[1]) {
                let stripped = line.trim();
                let Some((key, value)) = stripped.split_once(':') else {
                    continue;
                };
                metadata.insert(
                    key.trim().to_owned(),
                    value.trim().trim_matches('"').trim_matches('\'').to_owned(),
                );
            }
        }
    }

    (metadata, body.trim().to_owned())
}

/// `PROMPT_TEMPLATE_PATHS` (`agent.py:600`): the mode templates, resolved
/// against the process's base directory.
#[must_use]
pub fn prompt_template_paths(base_dir: &Path) -> BTreeMap<String, PathBuf> {
    [
        ("normal", "prompt_normal.md"),
        ("spooky", "prompt_spooky.md"),
        ("shakespearean", "prompt_shakespearean.md"),
        ("in-love", "prompt_in_love.md"),
    ]
    .into_iter()
    .map(|(mode, file)| ((*mode).to_owned(), base_dir.join("prompts").join(file)))
    .collect()
}

/// One `PROMPT_TEMPLATE_CACHE` (`agent.py:2136`) entry: the mtime the cached
/// template was read at, and the template.
type CacheEntry = (Option<f64>, PromptTemplate);

/// Python keys the cache by mode and keeps one process-wide dict; so does this.
type TemplateCache = Mutex<HashMap<String, CacheEntry>>;

fn template_cache() -> &'static TemplateCache {
    static CACHE: OnceLock<TemplateCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `path.stat().st_mtime if path.exists() else None` (`agent.py:2116`).
///
/// Python compares the two mtimes as floats; the float here is the same
/// seconds-since-epoch value `st_mtime` carries, so an untouched file hits and
/// a rewritten one misses exactly as the Python cache does.
fn file_mtime(path: &Path) -> Option<f64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs_f64())
}

/// `load_prompt_template` (`agent.py:2106`): the mode's template, re-read only
/// when its file's mtime moved.
///
/// The `Err` strings are the two `RuntimeError` texts the Python function
/// raises, verbatim; the read failure keeps Python's `: {exc}` shape but names
/// the error the way Rust's I/O layer spells it.
pub fn load_prompt_template(base_dir: &Path, mode: &str) -> Result<PromptTemplate, String> {
    // `str(mode or "spooky").strip().lower()`: an empty mode takes the default
    // before it is normalized — and lands on the same mode either way, since an
    // unknown one falls back below.
    //
    // The source's next two lines compare the lowered value against "SPOOKY",
    // which after `.lower()` it can never be. That branch is dropped rather
    // than carried over as dead code (docs/PARITY.md); nothing about the
    // resolved mode changes.
    let mut normalized = if mode.is_empty() {
        "spooky".to_owned()
    } else {
        mode.trim().to_lowercase()
    };

    let paths = prompt_template_paths(base_dir);
    if !paths.contains_key(normalized.as_str()) {
        normalized = "spooky".to_owned();
    }
    // `PROMPT_TEMPLATE_PATHS[normalized_mode]`: the fallback above guarantees
    // the mode is one of the four, so its path is always there.
    let path = match paths.get(normalized.as_str()) {
        Some(path) => path.clone(),
        None => {
            return Err(format!(
                "Prompt template path for mode '{normalized}' is missing"
            ))
        }
    };

    let current_mtime = file_mtime(&path);
    let mut cache = template_cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some((cached_mtime, template)) = cache.get(&normalized) {
        if *cached_mtime == current_mtime {
            return Ok(template.clone());
        }
    }

    if !path.exists() {
        return Err(format!(
            "Prompt template file for mode '{normalized}' was not found: {}",
            path.display()
        ));
    }

    // `path.read_text(encoding="utf-8")`: Python's reader strips a leading
    // BOM rather than folding it into the first character, and a BOM glued to
    // `---` would read as no frontmatter at all.
    let text = std::fs::read_to_string(&path)
        .map_err(|exc| format!("Failed to read prompt template for mode '{normalized}': {exc}"))?
        .trim_start_matches('\u{feff}')
        .to_owned();
    let (metadata, body) = parse_prompt_markdown(&text);
    if body.trim().is_empty() {
        return Err(format!(
            "Prompt template for mode '{normalized}' is empty: {}",
            path.display()
        ));
    }

    let template = PromptTemplate { metadata, body };
    cache.insert(normalized, (current_mtime, template.clone()));
    Ok(template)
}

/// `_prompt_allows_laugh` (`utils/prompt.py:115`): whether the mode's template
/// frontmatter carries `laugh: 1`.
///
/// The loader is a parameter, as it is in the Python helper — the real one is
/// `load_prompt_template` closed over the base directory, and a load failure
/// travels up exactly as the Python `RuntimeError` does. An unknown mode is
/// not a failure: it simply allows no laugh.
pub fn prompt_allows_laugh(
    mode: &str,
    template_paths: &BTreeMap<String, PathBuf>,
    load_template: impl Fn(&str) -> Result<PromptTemplate, String>,
) -> Result<bool, String> {
    let normalized = mode.trim().to_lowercase();
    if !template_paths.contains_key(normalized.as_str()) {
        return Ok(false);
    }

    let template = load_template(mode)?;
    Ok(template
        .metadata
        .get("laugh")
        .map_or("0", String::as_str)
        .trim()
        == "1")
}
