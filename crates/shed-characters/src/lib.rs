//! The character catalog: country files, picker rosters, identity rules, and
//! the persona a character profile is built from.
//!
//! The Python source spreads this over three places: `utils/characters.py`
//! (identity), a block of `agent.py` (`agent.py:1631`'s name table,
//! `agent.py:2196–2305`'s sex rules, `agent.py:2308–2570`'s country-file
//! caches and validation, `agent.py:2574–2692`'s roster, `agent.py:2707–2991`'s
//! profile builder and its helpers, `agent.py:2992–3086`'s system prompt, and
//! `agent.py:4690–4705`'s facts fallback). Plan §3.2 gathers them into this
//! one crate.
//!
//! | Python | Module |
//! |---|---|
//! | `utils/characters.py` | [`identity`] |
//! | `agent.py:1631`, `2196–2305` (sex) | [`sex`] |
//! | `agent.py:2308–2514` (country files, caches, validators) | [`catalog`] |
//! | `agent.py:2522–2692`, `2707–2732` (roster, labels) | [`picker`] |
//! | `agent.py:2735–2991`, `2992–3060`, `4690–4705` (persona, prompt, facts) | [`profile`] |
//!
//! The `str.title()`/`splitlines()` spellings these modules quote live in
//! `shed-text`'s `py_text`, next to the `repr()`/`str()` idioms in `value_text`
//! — `shed-prompts` needs them too and may not depend on this crate.

pub mod catalog;
pub mod identity;
pub mod picker;
pub mod profile;
pub mod sex;
