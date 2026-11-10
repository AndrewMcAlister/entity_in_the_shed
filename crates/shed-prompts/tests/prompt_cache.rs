//! The port of the pure half of `tests/test_prompt_cache.py` — the names,
//! the placeholders and the plan fields, none of which need an engine or an
//! HTTP mock.
//!
//! The classes that talk to `agent` (`StorePromptBlockTests`, `ProbeTests`,
//! `LmRequestFieldsTests`, `CombinedPromptTests`, the conversation-history
//! and retry suites) belong to the stages that own those routes. The digest
//! values quoted below are the ones Python computes — `shed-characters`
//! mints them, and this crate may not depend on that one, so the cross-check
//! is a constant rather than a call.

//! The test binary links `fancy_regex` and `shed_text` through the library
//! but names them never itself; the workspace's `unused_crate_dependencies`
//! lint judges the binary on its own uses.
use fancy_regex as _;
use shed_text as _;

use sha2::{Digest as _, Sha256};

use std::collections::BTreeMap;

use serde_json::{json, Value};
use shed_prompts::blocks::{
    body_names_missing_prompt_block, character_block_key, character_prompt_cache_id,
    expand_prompt_blocks, expand_prompt_blocks_text, is_prompt_block_listing, is_prompt_block_name,
    prompt_block_placeholder, user_info_block_key, PromptCachePlan,
};

/// `sha256("entity-character:" + "Canada:Siqiniq")` hex, first 16 — the id
/// `shed-characters` mints for that character, quoted from the Python side.
const SIQINIQ_CACHE_ID: &str = "e8ca9d6ec25664b4";

/// The same for `Testland:Aria` (`test_prompt_cache.py`'s `CHARACTER`).
const ARIA_CACHE_ID: &str = "0be1f85097f5414b";

/// A block mapping from name/value pairs, for the dicts the suite writes
/// as literals.
fn blocks(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// `CharacterCacheIdTests.test_the_id_is_a_usable_block_name_suffix`
/// (`test_prompt_cache.py:67`) — the id itself is guarded in
/// `shed-characters`; what belongs here is that the name built from it is
/// one Strata will take.
#[test]
fn the_id_is_a_usable_block_name_suffix() {
    assert_eq!(SIQINIQ_CACHE_ID.len(), 16);
    assert!(is_prompt_block_name(&character_block_key(SIQINIQ_CACHE_ID)));
}

/// `PromptBlockNameTests` (`test_prompt_cache.py:73`).
mod prompt_block_name {
    use super::*;

    #[test]
    fn a_conversation_block_is_named_for_the_conversation_and_character() {
        let key = user_info_block_key("client-abc123", "0123456789abcdef");

        assert!(is_prompt_block_name(&key));
        assert_eq!(
            key,
            user_info_block_key("client-abc123", "0123456789abcdef")
        );
        assert_ne!(key, user_info_block_key("client-other", "0123456789abcdef"));
        // The same browser talking to another character has another record, so
        // the other character's block must be another name.
        assert_ne!(
            key,
            user_info_block_key("client-abc123", "fedcba9876543210")
        );
    }

    #[test]
    fn an_address_key_never_reaches_the_block_name() {
        // Keys can be addresses or browser ids of any shape; the name is a
        // digest, so only letters and digits ever appear.
        let key = user_info_block_key("2001:db8::1", "0123456789abcdef");
        assert!(is_prompt_block_name(&key));
    }

    #[test]
    fn no_conversation_means_no_block() {
        assert_eq!(user_info_block_key("", "0123456789abcdef"), "");
        assert_eq!(user_info_block_key("   ", "0123456789abcdef"), "");
    }
}

/// `PromptCachePlanTests` (`test_prompt_cache.py:95`).
mod prompt_cache_plan {
    use super::*;

    #[test]
    fn referencing_a_block_records_its_text() {
        let mut plan = PromptCachePlan::new("client-abc", "");
        let placeholder = plan.reference("character_x", "the card");

        assert_eq!(placeholder, "{{cache:character_x}}");
        assert_eq!(plan.blocks, blocks(&[("character_x", "the card")]));
    }

    #[test]
    fn the_pin_is_sent_only_when_the_prompt_refers_to_it() {
        let mut plan = PromptCachePlan::new("client-abc", "character_x");
        assert_eq!(
            plan.request_fields(),
            blocks(&[("cache_client", "client-abc")])
        );

        plan.reference("character_x", "the card");
        assert_eq!(
            plan.request_fields(),
            blocks(&[("cache_pin", "character_x"), ("cache_client", "client-abc")])
        );
    }

    #[test]
    fn no_names_means_no_fields() {
        assert_eq!(PromptCachePlan::default().request_fields(), blocks(&[]));
    }
}

/// `ExpandTests` (`test_prompt_cache.py:117`).
mod expand {
    use super::*;

    #[test]
    fn a_placeholder_is_replaced_by_its_text() {
        assert_eq!(
            expand_prompt_blocks_text("a {{cache:x}} b", &blocks(&[("x", "MIDDLE")])),
            "a MIDDLE b"
        );
    }

    #[test]
    fn two_blocks_in_one_text_are_both_replaced() {
        assert_eq!(
            expand_prompt_blocks_text("{{cache:a}}{{cache:b}}", &blocks(&[("a", "1"), ("b", "2")])),
            "12"
        );
    }

    #[test]
    fn an_unknown_name_is_left_as_written() {
        assert_eq!(
            expand_prompt_blocks_text("{{cache:missing}}", &blocks(&[("x", "text")])),
            "{{cache:missing}}"
        );
    }

    #[test]
    fn messages_are_copied_rather_than_edited() {
        let original = json!([{"role": "system", "content": "{{cache:x}}"}]);
        let expanded = expand_prompt_blocks(&original, &blocks(&[("x", "text")]));

        assert_eq!(expanded, json!([{"role": "system", "content": "text"}]));
        assert_eq!(
            original,
            json!([{"role": "system", "content": "{{cache:x}}"}])
        );
    }

    #[test]
    fn a_refusal_about_a_block_is_recognised() {
        assert!(body_names_missing_prompt_block(
            r#"{"error": {"message": "no cache block named 'character_x' is stored"}}"#
        ));
        assert!(body_names_missing_prompt_block(
            "cache_pin: 'x' is not a cache block in this prompt"
        ));
        assert!(!body_names_missing_prompt_block("invalid model"));
    }

    #[test]
    fn the_placeholder_wears_the_trimmed_name() {
        // `prompt_block_placeholder` trims, so a key padded in a template
        // still names the block Strata stored under.
        assert_eq!(
            prompt_block_placeholder(" character_x "),
            "{{cache:character_x}}"
        );
    }
}

/// `BlockStoreShapeTests` (`test_prompt_cache.py:155`).
#[test]
fn stratas_listing_is_recognised_and_another_servers_body_is_not() {
    assert!(is_prompt_block_listing(&json!({"entries": [], "count": 0})));
    assert!(!is_prompt_block_listing(&json!({"data": []})));
    assert!(!is_prompt_block_listing(&json!([])));
    assert!(!is_prompt_block_listing(&Value::Null));
}

/// `CharacterFileIdTests` (`test_prompt_cache.py:165`), the two cases that
/// are about the name check rather than about `agent`'s profile builder.
mod character_file_id {
    use super::*;

    /// `test_prompt_cache.py:30`'s `CHARACTER`, the fixture profile.
    fn character() -> Value {
        json!({
            "name": "Aria",
            "locale": "en-GB",
            "character_id": "Testland:Aria",
            "cache_id": "0123456789abcdef",
            "slug": "testland-aria",
            "region": "Testland",
            "country": "Testland",
            "sex": "female",
        })
    }

    /// The derivation `shed-characters::identity::character_cache_id` runs
    /// over the profile's `character_id` — the closure the production wrapper
    /// passes to [`character_prompt_cache_id`]. The digest is quoted from the
    /// Python side because this crate may not call the other one.
    fn derived_id(profile: &Value) -> String {
        let identity = profile
            .get("character_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if identity.is_empty() {
            return String::new();
        }
        let digest = Sha256::digest(format!("entity-character:{identity}").as_bytes());
        let hex = format!("{digest:x}");
        hex[..16].to_owned()
    }

    /// The authored id is what makes one character's block reusable by every
    /// user who picks it, so a valid one is taken as written.
    #[test]
    fn an_authored_id_is_taken_as_written() {
        assert_eq!(
            character_prompt_cache_id(&character(), derived_id),
            "0123456789abcdef"
        );
    }

    /// `test_an_older_file_still_gets_an_id_from_its_identity`
    /// (`test_prompt_cache.py:191`).
    #[test]
    fn an_older_file_still_gets_an_id_from_its_identity() {
        let mut profile = character().as_object().cloned().unwrap_or_default();
        profile.insert("cache_id".to_owned(), json!(""));

        assert_eq!(
            character_prompt_cache_id(&Value::Object(profile), derived_id),
            ARIA_CACHE_ID
        );
    }

    /// The `isinstance(character_profile, dict)` half of `agent.py:3069`: a
    /// profile that is not an object derives from nothing, exactly as the
    /// empty dict Python collapses it to does.
    #[test]
    fn a_profile_that_is_not_an_object_derives_from_nothing() {
        assert_eq!(character_prompt_cache_id(&json!("Aria"), derived_id), "");
    }
}
