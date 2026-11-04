//! The model names the app is configured with.
//!
//! Ported from `utils/env.py:65` (`normalize_AI_MODEL_name`) and `agent.py:394-395`.

use crate::env::Env;

/// `normalize_AI_MODEL_name`: trim it, drop a trailing `:latest` only from a name that has
/// more than one colon.
///
/// A bare `model:latest` is a tag the registry needs; `registry:5000/model:latest` is the same
/// model spelled with a host, and the engine already resolves it. Counting colons is how the
/// two are told apart, so the count is checked before the suffix is removed.
///
/// Crate-internal for now. `agent.py` calls this helper six times: twice for the two settings
/// read here, and four more times (`6440`, `6487`, `6823`, `7973`) when a model is picked for a
/// turn. Those call sites belong to `shed-models`, and the helper becomes part of `shed-config`'s
/// public surface in that stage rather than before it (`How_To_Code` principle 16).
pub(crate) fn normalize_ai_model_name(value: Option<&str>, default: &str) -> String {
    let raw = value.unwrap_or_default().trim();
    let name = if raw.is_empty() { default } else { raw };

    if name.ends_with(":latest") && name.matches(':').count() > 1 {
        return name[..name.len() - ":latest".len()].to_string();
    }
    name.to_string()
}

/// The two names `/health` reports, read the way `agent.py` reads them: an unset or blank
/// setting is an empty name, not the helper's default.
#[derive(Debug, Clone)]
pub struct ModelNames {
    /// `AI_MODEL` (`agent.py:394`).
    pub ai_model: String,
    /// `LM_ENGINE_NORMAL_MODEL` (`agent.py:395`), the model a normal turn prefers.
    pub normal_model: String,
}

impl ModelNames {
    pub fn read(env: &Env) -> Self {
        Self {
            ai_model: normalize_ai_model_name(env.get("AI_MODEL"), ""),
            normal_model: normalize_ai_model_name(env.get("LM_ENGINE_NORMAL_MODEL"), ""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python's own default parameter (`utils/env.py:65`). No call site in the source uses it —
    /// all six pass `default=` explicitly — so the clone carries no default of its own and this
    /// value only exists here, to prove the helper does apply one when a caller passes it.
    const PYTHON_DEFAULT_MODEL: &str = "deepseek-coder-v2:latest";

    fn normalized(value: &str) -> String {
        normalize_ai_model_name(Some(value), PYTHON_DEFAULT_MODEL)
    }

    #[test]
    fn latest_is_stripped_only_from_a_name_with_more_than_one_colon() {
        assert_eq!(normalized("registry:5000/model:latest"), "registry:5000/model");
        assert_eq!(normalized("myrepo/my-model:latest"), "myrepo/my-model:latest");
        assert_eq!(normalized("model:latest"), "model:latest");
    }

    #[test]
    fn blank_value_uses_the_default_and_whitespace_is_trimmed() {
        assert_eq!(normalized("  "), PYTHON_DEFAULT_MODEL);
        assert_eq!(normalized("  abc  "), "abc");
        assert_eq!(
            normalize_ai_model_name(None, "x:y"),
            "x:y",
            "an absent setting uses the caller's default"
        );
    }

    #[test]
    fn the_config_defaults_to_no_model_of_its_own() {
        let names = ModelNames::read(&Env::with_values([
            ("AI_MODEL".to_string(), String::new()),
            (
                "LM_ENGINE_NORMAL_MODEL".to_string(),
                String::from("Qwen3-Coder-30B-A3B-Instruct-Q6_K"),
            ),
        ]));
        assert_eq!(names.ai_model, "");
        assert_eq!(names.normal_model, "Qwen3-Coder-30B-A3B-Instruct-Q6_K");

        let empty = ModelNames::read(&Env::default());
        assert_eq!(empty.ai_model, "");
        assert_eq!(empty.normal_model, "");
    }
}
