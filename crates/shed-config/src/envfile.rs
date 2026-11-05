//! The `.env` reader: python-dotenv's grammar (the U1-spike decision, plan §11).
//!
//! The operator's real `.env` is written to python-dotenv's grammar, and the
//! spike showed the dotenvy crate cannot read it: dotenvy rejects unquoted
//! values containing backslashes or spaces (`LMSTUDIO_MODELS_DIR=D:\Models`,
//! `DEFAULT_RESTRICTED_MODEL=Qwen3.5 9B (Q8 0) (32k)`) and silently corrupts
//! `STORAGE_ROOT=C:\Temp\…` because it treats `\` as an escape. python-dotenv
//! takes unquoted values verbatim, so this reader does the same.
//!
//! Supported grammar — the subset `.env.example` documents:
//! - blank lines and `#` comment lines; an optional `export ` prefix
//! - `KEY=value`; lines that are not a valid assignment are skipped, exactly
//!   as python-dotenv skips them
//! - unquoted values: verbatim except a trailing ` # comment` and surrounding
//!   whitespace; `${VAR}` / `${VAR:-default}` interpolation
//! - double-quoted values: escapes `\n \t \r \\ \" \'` plus interpolation;
//!   a backslash before `${` keeps it literal (python-dotenv's lookbehind)
//!   and unrecognised escapes such as `\$` keep their backslash verbatim
//! - single-quoted values: literal, no escapes, no interpolation
//! - multi-line quoted values are not supported; the shipped files never use
//!   one, and a lone opening quote reads to end of line
//!
//! Interpolation resolves `${VAR}` from earlier entries of the same file
//! first, then from the process environment — python-dotenv's
//! `dotenv_values` order. `lookup` is passed in so this module never touches
//! `std::env`; the names interpolated are whatever the operator wrote, not
//! keys the code reads, so they are deliberately outside the settings-gate
//! read-set.

use std::collections::BTreeMap;

/// Parse `.env` text into entries, file order irrelevant (BTreeMap).
pub fn parse(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    for line in text.lines() {
        if let Some((key, raw)) = split_assignment(line) {
            let value = match value_kind(raw) {
                Kind::Single(body) => body.to_owned(),
                Kind::Double(body) => interpolate(&unescape(body), lookup, &entries),
                Kind::Bare(body) => interpolate(strip_inline_comment(body).trim(), lookup, &entries),
            };
            entries.insert(key, value);
        }
    }
    entries
}

enum Kind<'a> {
    Single(&'a str),
    Double(&'a str),
    Bare(&'a str),
}

/// `KEY=rest` with the `export ` prefix gone; `None` for blanks, comments
/// and malformed lines. The value keeps its surrounding whitespace: whether
/// a `#` opens a comment depends on the whitespace before it (`C= # c` is an
/// empty value, `C=# c` is the literal `# c`), so trimming belongs to
/// [`value_kind`] and the bare path, not here.
fn split_assignment(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let rest = match trimmed.strip_prefix("export ") {
        Some(rest) => rest,
        None => trimmed,
    };
    let (key, value) = rest.split_once('=')?;
    let key = key.trim();
    if is_key(key) {
        Some((key.to_owned(), value))
    } else {
        None
    }
}

fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn value_kind(raw: &str) -> Kind<'_> {
    // Quote detection looks past leading whitespace; the bare path keeps it,
    // because strip_inline_comment needs the byte before any `#`.
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix('\'') {
        return Kind::Single(match rest.strip_suffix('\'') {
            Some(body) => body,
            None => rest,
        });
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        return Kind::Double(match rest.strip_suffix('"') {
            Some(body) => body,
            None => rest,
        });
    }
    Kind::Bare(raw)
}

/// A `#` starts a comment only after whitespace: `a=b#c` keeps `b#c`,
/// `a=b #c` yields `b` (python-dotenv's rule).
fn strip_inline_comment(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        let after_whitespace = index > 0 && matches!(bytes[index - 1], b' ' | b'\t');
        if *byte == b'#' && after_whitespace {
            return &raw[..index];
        }
    }
    raw
}

/// Double-quoted escape pass. python-dotenv's escape set is `[abfnrtv'"\\]`
/// plus numeric/unicode escapes; an unrecognised escape (`\$`, `\{`, …) keeps
/// its backslash verbatim, which is what the interpolation lookbehind below
/// relies on to leave `\${VAR}` unexpanded.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `${VAR}` and `${VAR:-default}`; an unresolved name with no fallback
/// becomes empty, as python-dotenv does. A `${` preceded by a backslash is
/// literal — python-dotenv's `(?<!\\)` lookbehind, which does not count
/// doubled backslashes, so neither does this.
fn interpolate(
    raw: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    entries: &BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find("${") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let literal = rest[..open].ends_with('\\');
        let close = match after.find('}') {
            Some(index) if !literal => index,
            _ => {
                out.push_str("${");
                rest = after;
                continue;
            }
        };
        let name_part = &after[..close];
        let (name, fallback) = match name_part.split_once(":-") {
            Some((name, fallback)) => (name, Some(fallback)),
            None => (name_part, None),
        };
        let resolved = match entries.get(name).cloned().or_else(|| lookup(name)) {
            Some(value) => value,
            None => match fallback {
                Some(fallback) => fallback.to_owned(),
                None => String::new(),
            },
        };
        out.push_str(&resolved);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::parse;
    use std::collections::BTreeMap;

    fn no_process_vars(_name: &str) -> Option<String> {
        None
    }

    fn process(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| owned.get(name).cloned()
    }

    #[test]
    fn plain_and_export_lines_and_comments() {
        let entries = parse(
            "# comment\n\n  # indented comment\nA=1\nexport B=2\nC = 3 \n",
            &no_process_vars,
        );
        assert_eq!(entries.get("A").map(String::as_str), Some("1"));
        assert_eq!(entries.get("B").map(String::as_str), Some("2"));
        assert_eq!(entries.get("C").map(String::as_str), Some("3"));
        assert_eq!(entries.len(), 3);
    }

    /// The exact shapes that sank dotenvy in the U1 spike: backslashes and
    /// spaces in unquoted values must survive verbatim.
    #[test]
    fn unquoted_values_are_verbatim() {
        let entries = parse(
            "LMSTUDIO_MODELS_DIR=D:\\Models\nSTORAGE_ROOT=C:\\Temp\\entity_in_the_shed\n\
             DEFAULT_RESTRICTED_MODEL=Qwen3.5 9B (Q8 0) (32k)\n",
            &no_process_vars,
        );
        assert_eq!(
            entries.get("LMSTUDIO_MODELS_DIR").map(String::as_str),
            Some("D:\\Models")
        );
        assert_eq!(
            entries.get("STORAGE_ROOT").map(String::as_str),
            Some("C:\\Temp\\entity_in_the_shed")
        );
        assert_eq!(
            entries.get("DEFAULT_RESTRICTED_MODEL").map(String::as_str),
            Some("Qwen3.5 9B (Q8 0) (32k)")
        );
    }

    #[test]
    fn inline_comments_need_leading_whitespace() {
        let entries = parse("A=b#c\nB=b #c\nC=   # c\n", &no_process_vars);
        assert_eq!(entries.get("A").map(String::as_str), Some("b#c"));
        assert_eq!(entries.get("B").map(String::as_str), Some("b"));
        assert_eq!(entries.get("C").map(String::as_str), Some(""));
    }

    #[test]
    fn quotes_and_escapes() {
        let entries = parse(
            "S='literal \\n no ${X} '\nD=\"tab\\there nl\\nend\"\nE=\"a\\$\\{X}b\"\n",
            &process(&[("X", "boom")]),
        );
        assert_eq!(
            entries.get("S").map(String::as_str),
            Some("literal \\n no ${X} ")
        );
        assert_eq!(entries.get("D").map(String::as_str), Some("tab\there nl\nend"));
        assert_eq!(entries.get("E").map(String::as_str), Some("a\\$\\{X}b"));
    }

    #[test]
    fn interpolation_file_first_then_process() {
        let entries = parse(
            "A=file\nB=${A}/${MISSING}/${MISSING:-fb}\nP=${FROM_PROCESS}!\n",
            &process(&[("FROM_PROCESS", "proc")]),
        );
        assert_eq!(entries.get("B").map(String::as_str), Some("file//fb"));
        assert_eq!(entries.get("P").map(String::as_str), Some("proc!"));
    }

    #[test]
    fn malformed_lines_are_skipped_like_python_dotenv() {
        let entries = parse("no equals\n9BAD=x\nGOOD=1\n", &no_process_vars);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.get("GOOD").map(String::as_str), Some("1"));
    }

    #[test]
    fn empty_value_is_kept() {
        let entries = parse("EMPTY=\n", &no_process_vars);
        assert_eq!(entries.get("EMPTY").map(String::as_str), Some(""));
    }
}
