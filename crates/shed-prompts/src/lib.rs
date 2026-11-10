//! Prompt building: the mode templates and the variables interpolated into
//! them, the language and tone guidance blocks, and the named prompt-cache
//! blocks Strata expands.
//!
//! Ported from `utils/prompt.py` and `utils/prompt_cache.py`, plus the parts
//! of `agent.py` that feed them. Only the helper a stage actually calls is
//! carried (`How_To_Code` principle 16), so this crate grows one function at
//! a time, in the stage that first needs it:
//!
//! | Python | Where it goes |
//! |---|---|
//! | `parse_prompt_markdown`, `_prompt_allows_laugh` (`utils/prompt.py:16/115`), `PROMPT_TEMPLATE_PATHS` and `load_prompt_template` (`agent.py:600/2106`) | here, in `template.rs` |
//! | `_list_field_text`, `_prompt_template_variables`, `render_prompt_variables` (`utils/prompt.py:36/48/103`) | here, in `variables.rs` — the profile facts the Python wrapper injects are a parameter, because the layer map keeps `shed-characters` out of this crate |
//! | `language_display_name`, `build_language_system_prompt` (`utils/prompt.py:128/136`) | here, in `language.rs` |
//! | `_build_tone_system_prompt` (`utils/prompt.py:149`) | here, in `tone.rs` — the settings map is a parameter, as it is in the Python helper |
//! | all of `utils/prompt_cache.py`, plus `character_prompt_cache_id`'s name check (`agent.py:3062`) | here, in `blocks.rs` — the derived-id half is a closure parameter, because minting it belongs to `shed-characters` |
//! | the rest of `agent.py:608-697` (`DELIVERY_TTS_DEFAULTS`, `TONE_TTS_SETTINGS`) | the stage that first needs the maps themselves — the chat prompt and the TTS pipeline; Stage 3's routes and suites never read them (the tone prompt takes its settings as an argument, exactly as the Python helper does) |

pub mod blocks;
pub mod language;
pub mod template;
pub mod tone;
pub mod variables;
