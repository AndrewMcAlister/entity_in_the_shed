//! Voice-depth coercion: forcing a number into the range the TTS engines accept.
//!
//! Ported from `agent.py:3267` (`clamp`), `agent.py:3275` (`normalize_voice_depth`),
//! `agent.py:2405` (`coerce_voice_depth`) and the `float(value)` idiom at
//! `agent.py:3269`/`2408`/`2434`.
//!
//! These live in `shed-text` rather than `shed-tts` for the same reason as
//! [`crate::value_text`]: the layer map (plan §3.3) lets `shed-tts` and
//! `shed-characters` depend on this crate but never on each other, and both
//! coerce authored pitches through the exact same rule — `shed-characters`
//! validates and serves them, `shed-tts` synthesises with them. Clamping them
//! separately would be two answers to "what depth does this pitch mean".
//! `shed-tts` re-exports them so its synthesis code keeps one import.

use serde_json::Value;

/// `clamp` (`agent.py:3267`): the value is already the `Option<f64>` a
/// `float()` would have produced ([`coerce_f64`]), so the `except` arm is the
/// `None` case.
///
/// `f64::min`/`f64::max` return the other operand when one is NaN, which is
/// what Python's `min(upper, nan)`/`max(lower, ...)` do too: `nan` clamps to
/// the upper bound, never to itself.
fn clamp(value: Option<f64>, lower: f64, upper: f64, fallback: f64) -> f64 {
    let numeric = value.unwrap_or(fallback);
    lower.max(upper.min(numeric))
}

/// `normalize_voice_depth` (`agent.py:3275`).
#[must_use]
pub fn normalize_voice_depth(value: Option<f64>, fallback: f64) -> f64 {
    clamp(value, 25.0, 130.0, fallback)
}

/// Python's `float(value)` over a JSON value (`agent.py:3269`, `agent.py:2408`,
/// `agent.py:2434`): numbers pass, booleans count as 1 and 0, numeric strings
/// parse, and everything else is the `TypeError`/`ValueError` the callers
/// catch — read here as `None`.
///
/// Rust's `f64::from_str` accepts the same `inf`/`infinity`/`nan` spellings
/// Python's `float()` does. Underscores are stripped before parsing, which
/// matches Python for every string a country file actually authors
/// (`"72.5"`, `" 72 "`); Python additionally rejects a trailing one (`"72_"`),
/// which no shipped file writes.
#[must_use]
pub fn coerce_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        Value::String(text) => {
            let cleaned: String = text.trim().chars().filter(|c| *c != '_').collect();
            cleaned.parse::<f64>().ok()
        }
        _ => None,
    }
}

/// `coerce_voice_depth` (`agent.py:2405`): an authored pitch, forced into the
/// valid range. `fallback` is the source's `fallback=72.0` parameter; the
/// `except` arm re-normalises it against 72.0 exactly as `agent.py:2410` does.
#[must_use]
pub fn coerce_voice_depth(value: Option<&Value>, fallback: f64) -> f64 {
    match value.map(coerce_f64) {
        Some(Some(numeric)) => normalize_voice_depth(Some(numeric), fallback),
        _ => normalize_voice_depth(Some(fallback), 72.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn depth_clamps_like_pythons_min_max_chain() {
        assert_eq!(normalize_voice_depth(Some(72.0), 60.0), 72.0);
        assert_eq!(normalize_voice_depth(Some(200.0), 60.0), 130.0);
        assert_eq!(normalize_voice_depth(Some(10.0), 60.0), 25.0);
        assert_eq!(normalize_voice_depth(None, 60.0), 60.0);
        // Python's `max(25.0, min(130.0, nan))` is 130.0 on this interpreter,
        // and Rust's min/max return the other NaN-free operand the same way.
        assert_eq!(normalize_voice_depth(Some(f64::NAN), 60.0), 130.0);
        assert_eq!(normalize_voice_depth(Some(f64::INFINITY), 60.0), 130.0);
        assert_eq!(normalize_voice_depth(Some(f64::NEG_INFINITY), 60.0), 25.0);
    }

    #[test]
    fn float_coercion_follows_pythons_float() {
        assert_eq!(coerce_f64(&json!(72)), Some(72.0));
        assert_eq!(coerce_f64(&json!(72.5)), Some(72.5));
        assert_eq!(coerce_f64(&json!(" 72 ")), Some(72.0));
        assert_eq!(coerce_f64(&json!("1_000")), Some(1000.0));
        assert_eq!(coerce_f64(&json!(true)), Some(1.0));
        assert_eq!(coerce_f64(&json!(false)), Some(0.0));
        assert_eq!(coerce_f64(&json!("bogus")), None);
        assert_eq!(coerce_f64(&json!(null)), None);
        assert_eq!(coerce_f64(&json!([])), None);
    }

    #[test]
    fn authored_pitch_falls_back_to_the_fallback_then_the_range() {
        assert_eq!(coerce_voice_depth(None, 72.0), 72.0);
        // An unparsable pitch re-normalises the fallback itself (`agent.py:2410`).
        assert_eq!(coerce_voice_depth(Some(&json!("bogus")), 80.0), 80.0);
        assert_eq!(coerce_voice_depth(Some(&json!("bogus")), 200.0), 130.0);
        // null reaches float() as None -> TypeError, same except arm.
        assert_eq!(coerce_voice_depth(Some(&json!(null)), 200.0), 130.0);
        assert_eq!(coerce_voice_depth(Some(&json!("72.5")), 72.0), 72.5);
    }
}
