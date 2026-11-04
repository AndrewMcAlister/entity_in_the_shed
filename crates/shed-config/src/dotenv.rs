//! The `.env` file, read the way `agent.py:299` reads it.
//!
//! `agent.py` calls `load_dotenv(BASE_DIR / ".env", override=True)`, which is
//! `python-dotenv`'s `parse_stream`. The clone implements the same grammar instead of reaching
//! for a crate, because the off-the-shelf Rust parsers disagree with the file this operator
//! actually has: `dotenvy` treats `LMSTUDIO_MODELS_DIR=D:\Models` as an illegal escape sequence
//! and fails the whole file, while `python-dotenv` keeps the backslash and carries on.
//! Principle 19 makes this file the single source of runtime configuration, so the reader has
//! to agree with it as it stands.
//!
//! ## The grammar, as `dotenv/parser.py` defines it
//!
//! A binding is: any whitespace and newlines, an optional `export` plus horizontal whitespace,
//! a key (`[^=#\s]+`, or a `'single quoted key'`), horizontal whitespace, `=`, more horizontal
//! whitespace, a value, an optional `# comment`, then end of line.
//!
//! - A key that starts with `#` is a comment line, not a setting.
//! - A value that starts with a quote is read up to the matching quote *across lines*. Inside a
//!   double-quoted value, `\\`, `\'`, `\"` and the `\a \b \f \n \r \t \v` escapes are decoded;
//!   inside a single-quoted value only `\\` and `\'` are, so `'a\nb'` stays two characters.
//! - An unquoted value is the rest of the line, with the first `\s+#` and everything after it
//!   removed, then trailing whitespace dropped. `value#nospace` keeps the hash because nothing
//!   precedes it; `value # comment` does not. Leading whitespace never reaches this step,
//!   because the `=` already swallowed it, which is why `KEY= #c` is the value `#c`.
//! - A line that does not fit the grammar is skipped, which is what `python-dotenv` does after
//!   printing "could not parse statement starting at line N" to stderr. A name with no `=` is a
//!   setting with no value, and `load_dotenv` writes nothing for such a setting, so the
//!   launcher's value for that name survives.
//! - A quote that does not close abandons its binding, and the parser resumes on the following
//!   line. When a quote further down the file does close it, everything between the two quotes
//!   is lost with the binding that opened it, because the quoted pattern spans line breaks.
//!
//! `${NAME}` inside a value is kept literally instead of expanded from the environment. That
//! `python-dotenv` behaviour is left out on purpose: an expanded value makes the configuration
//! depend on whatever the launcher happened to have set, which is the opposite of principle 19,
//! and no setting in the current `.env` uses it.

use std::fs;
use std::path::Path;

/// Every assignment in the file, in file order, so a name that appears twice ends up with the
/// last value the way `override=True` applies it to `os.environ`.
///
/// A file that cannot be read yields nothing: `.env` is optional and `agent.py` starts fine
/// without one.
pub(crate) fn dotenv_entries(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    Parser::new(&text).bindings()
}

/// A walk through the file text, one binding at a time.
struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    fn bindings(&mut self) -> Vec<(String, String)> {
        let mut bindings = Vec::new();
        while self.at < self.text.len() {
            self.skip_whitespace_and_newlines();
            if self.at >= self.text.len() {
                break;
            }
            self.skip_export();
            match self.binding() {
                Some((name, value)) => bindings.push((name, value)),
                None => self.skip_line(),
            }
        }
        bindings
    }

    /// One `KEY=value`, or `None` when the text here is a comment line or does not fit.
    fn binding(&mut self) -> Option<(String, String)> {
        let name = self.read_key()?;
        self.skip_horizontal();
        if self.peek() != Some('=') {
            // A name with no `=` is a setting with no value, and `load_dotenv` writes nothing
            // for a value of `None`, so the launcher's value for that name survives.
            return None;
        }
        self.at += 1;
        self.skip_horizontal();

        let value = self.read_value()?;
        self.skip_horizontal();
        if self.peek() == Some('#') {
            self.skip_line();
        } else {
            self.finish_line()?;
        }
        Some((name, value))
    }

    /// `parse_key`: a comment line has no key, and an ordinary key stops at `=`, `#` or space.
    fn read_key(&mut self) -> Option<String> {
        if self.peek() == Some('#') {
            return None;
        }
        if self.peek() == Some('\'') {
            // `_single_quoted_key` is `'([^']+)'`: everything up to the next quote, with no
            // escapes, and no match at all when there is no closing quote.
            let opening = self.at;
            self.at += 1;
            let start = self.at;
            while let Some(ch) = self.peek() {
                if ch == '\'' {
                    let key = self.text[start..self.at].to_string();
                    self.at += 1;
                    return Some(key);
                }
                self.at += ch.len_utf8();
            }
            self.at = opening;
            return None;
        }

        let start = self.at;
        while let Some(ch) = self.peek() {
            if ch == '=' || ch == '#' || ch.is_whitespace() {
                break;
            }
            self.at += ch.len_utf8();
        }
        if self.at == start {
            return None;
        }
        Some(self.text[start..self.at].to_string())
    }

    /// `parse_value`, including the quoted branch that runs over line breaks.
    ///
    /// `None` is the unclosed-quote case, where `python-dotenv` abandons the whole binding.
    fn read_value(&mut self) -> Option<String> {
        match self.peek() {
            Some('\'') => self.quoted('\'').map(|inner| decode(&inner, false)),
            Some('"') => self.quoted('"').map(|inner| decode(&inner, true)),
            Some(ch) if ch == '\n' || ch == '\r' => Some(String::new()),
            None => Some(String::new()),
            Some(_) => Some(self.unquoted_value()),
        }
    }

    /// `parse_unquoted_value`: the rest of the line, minus the first comment, minus trailing
    /// whitespace.
    fn unquoted_value(&mut self) -> String {
        let start = self.at;
        self.skip_to_line_end();

        let part = &self.text[start..self.at];
        // `re.sub(r"\s+#.*", "", part).rstrip()`: the leftmost hash that whitespace precedes
        // starts a comment that runs to the end of the line. A hash with nothing before it is
        // part of the value, and the search carries on past it.
        let comment = part.char_indices().find(|(at, ch)| {
            *ch == '#' && part[..*at].chars().next_back().is_some_and(char::is_whitespace)
        });

        match comment {
            Some((at, _)) => part[..part[..at].trim_end().len()].trim_end().to_string(),
            None => part.trim_end().to_string(),
        }
    }

    /// A quoted run, or `None` when the quote never closes.
    ///
    /// Backslash escapes are kept raw for `decode` to handle, so a backslash cannot be mistaken
    /// for the escape of the quote that ends the run. That is also what the Python pattern does:
    /// `(?:\\.|[^"\\])*` steps over an escaped character whole.
    ///
    /// A quote that never closes abandons the binding, and the cursor is put back on the opening
    /// quote so the caller can skip the rest of that line, which is how `python-dotenv` recovers
    /// when there is no later quote for the pattern to reach.
    fn quoted(&mut self, quote: char) -> Option<String> {
        let opening = self.at;
        self.at += 1;
        let mut inner = String::new();
        while let Some(ch) = self.peek() {
            if ch == quote {
                self.at += 1;
                return Some(inner);
            }
            if ch == '\\' {
                inner.push('\\');
                self.at += 1;
                if let Some(escaped) = self.peek() {
                    inner.push(escaped);
                    self.at += escaped.len_utf8();
                }
                continue;
            }
            inner.push(ch);
            self.at += ch.len_utf8();
        }
        self.at = opening;
        None
    }

    /// `export` is allowed in front of a key, as `python-dotenv` allows it.
    fn skip_export(&mut self) {
        if !self.text[self.at..].starts_with("export") {
            return;
        }
        let after = self.at + "export".len();
        let Some(ch) = self.text[after..].chars().next() else {
            return;
        };
        if ch.is_whitespace() && ch != '\n' && ch != '\r' {
            self.at = after;
            self.skip_horizontal();
        }
    }

    /// `_end_of_line`: a binding only stands if nothing but whitespace follows it on its line.
    fn finish_line(&mut self) -> Option<()> {
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                self.at += 1;
                return Some(());
            }
            if ch == '\r' {
                self.at += 1;
                if self.peek() == Some('\n') {
                    self.at += 1;
                }
                return Some(());
            }
            if ch.is_whitespace() {
                self.at += ch.len_utf8();
                continue;
            }
            return None;
        }
        Some(())
    }

    /// `_rest_of_line`, taken when a binding is discarded.
    fn skip_line(&mut self) {
        self.skip_to_line_end();
        self.take_line_break();
    }

    /// `[^\r\n]*`: everything up to the next line break, without taking the break.
    fn skip_to_line_end(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' || ch == '\r' {
                return;
            }
            self.at += ch.len_utf8();
        }
    }

    /// One line break, `\r\n` counted as a single one.
    fn take_line_break(&mut self) {
        match self.peek() {
            Some('\r') => {
                self.at += 1;
                if self.peek() == Some('\n') {
                    self.at += 1;
                }
            }
            Some('\n') => self.at += 1,
            _ => {}
        }
    }

    /// `[^\S\r\n]*`: spaces and tabs, but not a line break.
    fn skip_horizontal(&mut self) {
        while let Some(ch) = self.peek() {
            if !ch.is_whitespace() || ch == '\n' || ch == '\r' {
                return;
            }
            self.at += ch.len_utf8();
        }
    }

    /// `\s*`: whitespace including line breaks, which is how a blank line is passed over.
    fn skip_whitespace_and_newlines(&mut self) {
        while let Some(ch) = self.peek() {
            if !ch.is_whitespace() {
                return;
            }
            self.at += ch.len_utf8();
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }
}

/// `decode_escapes`: inside double quotes, `\\ \' \" \a \b \f \n \r \t \v` are decoded and
/// anything else is left exactly as written; inside single quotes, only `\\` and `\'`.
fn decode(inner: &str, double: bool) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(escaped) = chars.next() else {
            out.push('\\');
            continue;
        };
        let decoded = match escaped {
            '\\' | '\'' | '"' => Some(escaped),
            'a' if double => Some('\u{7}'),
            'b' if double => Some('\u{8}'),
            'f' if double => Some('\u{12}'),
            'n' if double => Some('\n'),
            'r' if double => Some('\r'),
            't' if double => Some('\t'),
            'v' if double => Some('\u{11}'),
            _ => None,
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                out.push('\\');
                out.push(escaped);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each case writes its own probe file under the approved temporary tree, so the suite
    /// leaves nothing in the repository (principle 20) and the cases cannot collide while cargo
    /// runs them in parallel.
    fn parsed(label: &str, text: &str) -> Vec<(String, String)> {
        let dir = std::env::temp_dir().join("shed-config-dotenv-tests");
        let path = dir.join(format!("probe-{label}.env"));
        assert!(
            fs::create_dir_all(&dir).is_ok() && fs::write(&path, text).is_ok(),
            "{} must be writable",
            path.display()
        );
        dotenv_entries(&path)
    }

    /// The value a name ends up with, which is the last line that sets it: `override=True`
    /// applies the file in order, so a later line replaces an earlier one.
    fn value_of(entries: &[(String, String)], name: &str) -> String {
        entries
            .iter()
            .rev()
            .find(|(entry, _)| entry == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{name} was not read: {entries:?}"))
    }

    #[test]
    fn comments_and_blank_lines_are_not_settings() {
        let entries =
            parsed("comments", "# a comment\n   # an indented comment\n\n   \nKEY=value\n");
        assert_eq!(entries, vec![("KEY".to_string(), "value".to_string())]);
    }

    #[test]
    fn an_unquoted_value_keeps_inner_spaces_and_drops_the_ones_at_the_ends() {
        let entries =
            parsed("spaces", "A=hello world\nB=  leading\nC=trailing   \nK   =   both   \n");
        assert_eq!(value_of(&entries, "A"), "hello world");
        assert_eq!(value_of(&entries, "B"), "leading");
        assert_eq!(value_of(&entries, "C"), "trailing");
        assert_eq!(value_of(&entries, "K"), "both");
    }

    #[test]
    fn a_hash_starts_a_comment_only_when_whitespace_precedes_it() {
        let entries = parsed(
            "hash",
            "A=value # comment\nB=value#nospace\nC=100 # inline\nD=# whole value\nE= #c\nF=x#y #z\nG=value\t#tab\n",
        );
        assert_eq!(value_of(&entries, "A"), "value");
        assert_eq!(value_of(&entries, "B"), "value#nospace");
        assert_eq!(value_of(&entries, "C"), "100");
        // The `=` swallows the whitespace, so a hash that looks guarded is not guarded here.
        assert_eq!(value_of(&entries, "D"), "# whole value");
        assert_eq!(value_of(&entries, "E"), "#c");
        assert_eq!(value_of(&entries, "F"), "x#y");
        assert_eq!(value_of(&entries, "G"), "value");
    }

    #[test]
    fn a_windows_path_survives_untouched() {
        let entries = parsed(
            "paths",
            "LMSTUDIO_MODELS_DIR=D:\\Models\nSTORAGE_ROOT=D:\\\\temp\nTTS_CACHE=C:\\Users\\x\n",
        );
        assert_eq!(value_of(&entries, "LMSTUDIO_MODELS_DIR"), "D:\\Models");
        assert_eq!(value_of(&entries, "STORAGE_ROOT"), "D:\\\\temp");
        assert_eq!(value_of(&entries, "TTS_CACHE"), "C:\\Users\\x");
    }

    #[test]
    fn a_quoted_value_protects_a_hash_and_a_double_quote_decodes_the_c_style_escapes() {
        let entries = parsed(
            "quoted",
            "A=\"a # b\"\nB=\"trailing spaces  \"\nC=\"a\\\"b\"\nD='a\\'b'\nE=\"a\\nb\"\nF=\"a\\tb\"\nG=\"a\\\\b\"\nH='a\\nb'\nI=\"a\\1b\"\n",
        );
        assert_eq!(value_of(&entries, "A"), "a # b");
        assert_eq!(value_of(&entries, "B"), "trailing spaces  ");
        assert_eq!(value_of(&entries, "C"), "a\"b");
        assert_eq!(value_of(&entries, "D"), "a'b");
        assert_eq!(value_of(&entries, "E"), "a\nb");
        assert_eq!(value_of(&entries, "F"), "a\tb");
        assert_eq!(value_of(&entries, "G"), "a\\b");
        // Single quotes decode only `\\` and `\'`, so `\n` stays two characters there.
        assert_eq!(value_of(&entries, "H"), "a\\nb");
        assert_eq!(value_of(&entries, "I"), "a\\1b");
    }

    #[test]
    fn a_quoted_value_can_span_lines_and_a_comment_follows_the_closing_quote() {
        let entries = parsed("spanning", "A=\"multi\nline\"\nB=\"q\" # trailing comment\nC=next\n");
        assert_eq!(value_of(&entries, "A"), "multi\nline");
        assert_eq!(value_of(&entries, "B"), "q");
        assert_eq!(value_of(&entries, "C"), "next");
    }

    #[test]
    fn an_unclosed_quote_is_dropped_and_the_next_line_still_reads() {
        let entries = parsed("unclosed", "A=\"unclosed\nB=next\n");
        assert_eq!(entries, vec![("B".to_string(), "next".to_string())]);
    }

    #[test]
    fn a_later_quote_is_swallowed_by_an_unclosed_one_because_the_pattern_runs_across_lines() {
        // `C=` closes the quote `A=` opened, so `B` is lost with the binding that opened it.
        // The single-quoted `F=` has no closing quote anywhere after it, so only its own line
        // goes. `python-dotenv` loses these same lines and keeps these same survivors.
        let entries = parsed(
            "swallowed",
            "A=\"unclosed\nB=next\nC=\"open\nOTHER=1\nD=\"close\"\nE=after\nF='sq unclosed\nG=tail\n",
        );
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["OTHER", "D", "E", "G"]);
        assert_eq!(value_of(&entries, "D"), "close");
    }

    #[test]
    fn export_is_accepted_and_an_indented_key_still_reads() {
        let entries = parsed("export", "export FLASK_PORT=5005\n   FLASK_HOST=127.0.0.1\n");
        assert_eq!(value_of(&entries, "FLASK_PORT"), "5005");
        assert_eq!(value_of(&entries, "FLASK_HOST"), "127.0.0.1");
    }

    #[test]
    fn a_value_can_contain_an_equals_sign_and_a_later_line_wins() {
        let entries = parsed("repeated", "A=a=b\nA=second\n");
        assert_eq!(value_of(&entries, "A"), "second");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn a_line_that_does_not_fit_the_grammar_is_skipped_and_the_file_carries_on() {
        // `python-dotenv` prints "could not parse statement starting at line N" for each of
        // these and drops only that line. Trailing text after a closing quote is the one that is
        // easy to write by accident.
        let entries = parsed(
            "skipped",
            "GOOD1=1\nBAD LINE\n=noleadkey\nBROKEN='it''s'\nGOOD2=2\nC=\"closed\" junk\nGOOD3=3\n",
        );
        assert_eq!(value_of(&entries, "GOOD1"), "1");
        assert_eq!(value_of(&entries, "GOOD2"), "2");
        assert_eq!(value_of(&entries, "GOOD3"), "3");
        assert_eq!(entries.len(), 3, "{entries:?}");
    }

    #[test]
    fn a_key_may_start_with_a_digit_because_the_grammars_only_rule_is_no_equals_no_hash_no_space() {
        // `_unquoted_key` is `[^=#\s]+`, so `1NOPE=1` is a setting as far as `python-dotenv` is
        // concerned. The parser keeps it rather than inventing a stricter rule; principle 24 is
        // enforced by the settings this app actually reads, every one of which it asks for by
        // name.
        let entries = parsed("digit-key", "1NOPE=1\n_UNDER=1\nA=second\n");
        assert_eq!(value_of(&entries, "1NOPE"), "1");
        assert_eq!(value_of(&entries, "_UNDER"), "1");
    }

    #[test]
    fn a_bare_name_without_an_equals_leaves_the_launchers_value_alone() {
        // `foo` alone parses as a setting with no value, and `load_dotenv` writes nothing for a
        // `None` value, so the launcher's `foo` survives. Skipping the entry here is the same
        // thing.
        let entries = parsed("bare", "BARE\nGOOD=1\n");
        assert_eq!(entries, vec![("GOOD".to_string(), "1".to_string())]);
    }

    #[test]
    fn substitution_is_left_alone() {
        let entries = parsed("substitution", "A=${OTHER}/x\nB=%TEMP%\n");
        assert_eq!(value_of(&entries, "A"), "${OTHER}/x");
        assert_eq!(value_of(&entries, "B"), "%TEMP%");
    }

    #[test]
    fn a_missing_file_reads_as_no_settings() {
        assert!(dotenv_entries(Path::new("Z:\\definitely-not-here\\.env")).is_empty());
    }
}
