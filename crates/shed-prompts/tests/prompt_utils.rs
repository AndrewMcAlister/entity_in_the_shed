//! Rule A tests for the prompt helpers — the port of
//! `tests/test_prompt_utils.py`.
//!
//! Every class of the Python suite lands in the module that owns the
//! function it exercises: the template files' parsing and laugh flag in
//! [`template`], the interpolation variables in [`variables`], the language
//! block in [`language`], the tone block in [`tone`].

//! The test binary links these crates through the library but names none of
//! them itself; the workspace's `unused_crate_dependencies` lint judges the
//! binary on its own uses.
use fancy_regex as _;
use sha2 as _;
use shed_text as _;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};
use shed_prompts::language::{build_language_system_prompt, language_display_name};
use shed_prompts::template::{parse_prompt_markdown, prompt_allows_laugh, PromptTemplate};
use shed_prompts::tone::build_tone_system_prompt;
use shed_prompts::variables::{prompt_template_variables, render_prompt_variables};

/// `prompt._FACTS(profile)` (`test_prompt_utils.py:32`): the persona facts a
/// male character would resolve to.
fn facts() -> Value {
    json!({
        "subject_pronoun": "he",
        "object_pronoun": "him",
        "possessive_adjective": "his",
        "reflexive_pronoun": "himself",
        "origin_detail": "raised in Lyon",
    })
}

/// A `BTreeMap` from name/value pairs, for the maps the Python suite writes
/// as literals.
fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// `TestParsePromptMarkdown` (`test_prompt_utils.py:5`).
mod parse_prompt_markdown {
    use super::*;

    #[test]
    fn frontmatter_with_quoted_values() {
        let text = "---\n\
                    name: \"Greg\"\n\
                    region: France\n\
                    quote: 'quoted'\n\
                    # a comment line\n\
                    no colon here\n\
                    ---\n\
                    Hello body";
        let (metadata, body) = parse_prompt_markdown(text);
        assert_eq!(
            metadata,
            map(&[("name", "Greg"), ("region", "France"), ("quote", "quoted")])
        );
        assert_eq!(body, "Hello body");
    }

    #[test]
    fn no_frontmatter() {
        let (metadata, body) = parse_prompt_markdown("just body");
        assert_eq!(metadata, map(&[]));
        assert_eq!(body, "just body");
    }

    #[test]
    fn empty_text() {
        let (metadata, body) = parse_prompt_markdown("");
        assert_eq!(metadata, map(&[]));
        assert_eq!(body, "");
    }
}

/// `TestPromptTemplateVariables` (`test_prompt_utils.py:42`).
mod prompt_template_variables {
    use super::*;

    /// `TestPromptTemplateVariables.PROFILE` (`test_prompt_utils.py:43`).
    fn profile() -> Value {
        json!({
            "name": "Testy",
            "country": "France",
            "mannerisms": ["quirky", "calm"],
            "backstory": "l1",
            "backstory_lines": ["l1", "l2"],
            "region": "Paris",
            "locale": "fr-FR",
            "voice_style": "warm",
            "political_temperament": "centrist",
            "speech_register": ["drop the 'h'", "say 'me' for 'my'"],
        })
    }

    #[test]
    fn full_profile() {
        let variables =
            prompt_template_variables(Some(&profile()), &json!("what is your name?"), &facts());
        assert_eq!(variables["Name"], "Testy");
        assert_eq!(variables["name"], "Testy");
        assert_eq!(variables["Country"], "France");
        assert_eq!(variables["country"], "France");
        assert_eq!(variables["Mannerisms"], "quirky; calm");
        assert_eq!(
            variables["SpeechRegister"],
            "drop the 'h'; say 'me' for 'my'"
        );
        assert_eq!(
            variables["speech_register"],
            "drop the 'h'; say 'me' for 'my'"
        );
        assert_eq!(variables["Backstory"], "l1");
        assert_eq!(variables["BackstoryLines"], "l1\nl2");
        assert_eq!(variables["HomeRegion"], "Paris");
        assert_eq!(variables["OriginDetail"], "raised in Lyon");
        assert_eq!(variables["Locale"], "fr-FR");
        assert_eq!(variables["Region"], "Paris");
        assert_eq!(variables["VoiceStyle"], "warm");
        assert_eq!(variables["PoliticalTemperament"], "centrist");
        assert_eq!(variables["input"], "what is your name?");
    }

    #[test]
    fn sex_and_pronouns_titled() {
        let mut profile = profile().as_object().cloned().unwrap_or_default();
        profile.insert("sex".to_owned(), json!("male"));
        let variables =
            prompt_template_variables(Some(&Value::Object(profile)), &json!(""), &facts());
        assert_eq!(variables["Sex"], "Male");
        assert_eq!(variables["sex"], "male");
        assert_eq!(variables["SubjectPronoun"], "He");
        assert_eq!(variables["ObjectPronoun"], "Him");
        assert_eq!(variables["PossessiveAdjective"], "His");
        assert_eq!(variables["ReflexivePronoun"], "Himself");
    }

    #[test]
    fn empty_profile_defaults() {
        let variables = prompt_template_variables(Some(&json!({})), &json!(""), &json!({}));
        assert_eq!(variables["Name"], "Greg");
        assert_eq!(variables["country"], "the English-speaking world");
        assert_eq!(variables["Sex"], "Unknown");
        // Most characters author no register; the mode template interpolates
        // this unconditionally, so it has to degrade to an empty string rather
        // than the word "None".
        assert_eq!(variables["speech_register"], "");
        assert_eq!(variables["SpeechRegister"], "");
        assert_eq!(variables["SubjectPronoun"], "They");
        assert_eq!(variables["ObjectPronoun"], "Them");
        assert_eq!(variables["PossessiveAdjective"], "Their");
        assert_eq!(variables["ReflexivePronoun"], "Themself");
        assert_eq!(variables["input"], "");
    }
}

/// `TestRenderPromptVariables` (`test_prompt_utils.py:105`).
mod render_prompt_variables {
    use super::*;

    #[test]
    fn replaces_braces_and_brackets() {
        let rendered = render_prompt_variables(
            &json!("Hello {{Name}} from {country}."),
            &map(&[("Name", "Greg"), ("country", "France")]),
        );
        assert_eq!(rendered, "Hello Greg from France.");
    }

    #[test]
    fn unknown_key_replaced_with_empty() {
        assert_eq!(
            render_prompt_variables(&json!("a {missing} b"), &map(&[])),
            "a  b"
        );
    }

    #[test]
    fn collapses_multiple_newlines() {
        assert_eq!(
            render_prompt_variables(&json!("a\n\n\n\nb"), &map(&[])),
            "a\n\nb"
        );
    }

    #[test]
    fn strips_outer_whitespace() {
        assert_eq!(render_prompt_variables(&json!("  hi  "), &map(&[])), "hi");
    }

    #[test]
    fn empty_text() {
        assert_eq!(render_prompt_variables(&json!(""), &map(&[])), "");
    }
}

/// `TestPromptAllowsLaugh` (`test_prompt_utils.py:125`).
mod prompt_allows_laugh {
    use super::*;

    /// `paths = {"normal": "path/normal.md"}` — the mode table the suite
    /// patches in place of the real four modes.
    fn paths() -> BTreeMap<String, PathBuf> {
        BTreeMap::from([("normal".to_owned(), PathBuf::from("path/normal.md"))])
    }

    /// `lambda mode: {"metadata": metadata}` — the loader the suite injects.
    fn loader(
        metadata: BTreeMap<String, String>,
    ) -> impl Fn(&str) -> Result<PromptTemplate, String> {
        move |_mode| {
            Ok(PromptTemplate {
                metadata: metadata.clone(),
                body: String::new(),
            })
        }
    }

    #[test]
    fn mode_present_and_laugh_enabled() {
        let allowed = prompt_allows_laugh("normal", &paths(), loader(map(&[("laugh", "1")])));
        assert_eq!(allowed, Ok(true));
    }

    #[test]
    fn mode_not_in_paths() {
        let allowed = prompt_allows_laugh("spooky", &paths(), loader(map(&[("laugh", "1")])));
        assert_eq!(allowed, Ok(false));
    }

    #[test]
    fn no_laugh_key() {
        let allowed = prompt_allows_laugh("normal", &paths(), loader(map(&[])));
        assert_eq!(allowed, Ok(false));
    }

    #[test]
    fn blank_mode() {
        let allowed = prompt_allows_laugh("", &paths(), loader(map(&[("laugh", "1")])));
        assert_eq!(allowed, Ok(false));
    }
}

/// `TestLanguageDisplayName` (`test_prompt_utils.py:147`).
mod language_display_name {
    use super::*;

    #[test]
    fn french() {
        assert_eq!(language_display_name("fr-FR"), "French");
    }

    #[test]
    fn japanese() {
        assert_eq!(language_display_name("ja"), "Japanese");
    }

    #[test]
    fn default_english() {
        assert_eq!(language_display_name(""), "English");
    }

    #[test]
    fn unsupported_falls_back_to_english() {
        assert_eq!(language_display_name("xx-YY"), "English");
    }
}

/// `TestBuildLanguageSystemPrompt` (`test_prompt_utils.py:161`).
mod build_language_system_prompt {
    use super::*;

    #[test]
    fn default() {
        let result = build_language_system_prompt("");
        assert!(result.contains("Conversation language guidance:"));
        assert!(result.contains("English (en-GB)"));
    }

    #[test]
    fn french() {
        let result = build_language_system_prompt("fr-FR");
        assert!(result.contains("French (fr-FR)"));
        assert!(result.contains("- Reply in French by default."));
    }
}

/// `TestBuildToneSystemPrompt` (`test_prompt_utils.py:173`).
mod build_tone_system_prompt {
    use super::*;

    /// A settings map like the suite's literals: each tone either carries a
    /// description or is bare rate/pitch data.
    fn tones(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(name, description)| {
                let tone = match description {
                    Some(text) => json!({"description": text}),
                    None => json!({}),
                };
                ((*name).to_owned(), tone)
            })
            .collect()
    }

    #[test]
    fn sorted_lines_with_descriptions() {
        let result = build_tone_system_prompt(&tones(&[
            ("brave", Some("Bold and daring")),
            ("angry", Some("Hot and loud")),
        ]));
        assert_eq!(
            result,
            "Available voice tone tags:\n- angry: Hot and loud\n- brave: Bold and daring"
        );
    }

    #[test]
    fn tone_without_description() {
        let result = build_tone_system_prompt(&tones(&[("calm", None)]));
        assert_eq!(result, "Available voice tone tags:\n- calm");
    }

    #[test]
    fn empty_settings() {
        assert_eq!(
            build_tone_system_prompt(&BTreeMap::new()),
            "Available voice tone tags:"
        );
    }
}
