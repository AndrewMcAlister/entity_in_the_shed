//! Repository-level gates for the Rust clone of The Entity In The Shed.
//!
//! This crate owns no domain logic and no shipped crate depends on it. Each module states
//! one invariant that spans the whole workspace and therefore belongs to no single domain
//! crate, and carries the tests that enforce it:
//!
//! - [`assets`] - decision D2: the browser client, the data it reads and the eight browser
//!   suites are copied byte-for-byte and frozen.
//! - [`layers`] - plan 3.3: the crate dependency direction, and the rule that only
//!   `shed-config` reads the environment (`How_To_Code` principles 19 and 24).
//! - [`settings`] - principle 24: `.env.example` documents exactly the settings the code reads,
//!   with the defaults the code actually uses.

use std::path::{Path, PathBuf};

pub mod assets;
pub mod layers;
pub mod settings;

/// A gate test that reports every problem it found rather than only the first one, so one
/// run tells the whole story.
pub type Gate = Result<(), Box<dyn std::error::Error>>;

/// This package lives at `crates/shed-gates`, so the repository root is two levels up.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
