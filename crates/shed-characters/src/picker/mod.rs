//! The picker's view of a country: the roster entries it offers, and the
//! names and labels those entries spell out.
//!
//! `country_roster` itself is a method on
//! [`CharacterCatalog`](crate::catalog::CharacterCatalog), declared here so
//! the roster reads the country files through the same signature caches.

mod labels;
mod roster;

pub use labels::{
    character_label_marker, disambiguate_labels, slugify_character_name, voice_human_name,
};
pub use roster::{character_dedupe_key, character_details, character_text_lines};
