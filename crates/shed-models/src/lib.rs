//! The model catalog: what the app believes is on the machine, how it learned that, and where
//! it keeps that knowledge between boots.
//!
//! Ported from `agent.py`'s discovery block (1239-1629) and from `utils/models.py`, which is the
//! same rules lifted out of `agent.py` and re-imported back as a facade so the old call sites
//! keep working.
//!
//! ## The one dynamic value this crate touches
//!
//! `serde_json::Value` arrives here exactly once, as the discovery document an engine answers
//! with. Plan §6 bans `Value` outside `shed-json` and the MCP argument layer, and this is the
//! exception recorded there: an engine's model list is genuinely dynamic.
//! `GET {LM_ENGINE_HOST}/v1/models` is answered by whatever LM Studio build the operator happens
//! to be running, `parse_discovery_candidates` (`utils/models.py:117`) accepts five container
//! shapes, and `ingest_discovery_item` (`agent.py:1354`) accepts six spellings of a model's name.
//! A typed request struct would reject the whole document where the source's defensive
//! `isinstance` and `try` accept it: LM Studio spells a variant `"q4_0"` in one build and
//! `{"file": ...}` in another.
//!
//! `Value` is consumed inside `discovery.rs` and does not leave this crate. What the catalog
//! holds afterwards is `ModelEntry` and `ModelMetadata`, both typed.
//!
//! ## What is here, and what is waiting for its first caller
//!
//! | Source | Where it goes |
//! |---|---|
//! | `parse_model_size_billions` (`utils/models.py:10`) | here, in `size.rs` |
//! | `_normalize_model_metadata_map` (`utils/models.py:21`) | here, in `catalog.rs` |
//! | `parse_discovery_candidates` (`utils/models.py:117`) | here, in `discovery.rs` |
//! | `filter_embedding_model_entries` (`utils/models.py:134`), `model_discovery_size_sort_key` (152) | here, in `entry.rs` |
//! | `ingest_discovery_item` (`agent.py:1354`) | here, in `discovery.rs` |
//! | `discover_lm_studio_models` (`agent.py:1529`) | split: its HTTP half is `shed-llm`'s `discovery`, its catalog half is `apply_discovered_payload` here, because `LAYERS` lets `shed-llm` reach `shed-models` and never the reverse |
//! | `_apply_known_models` (`agent.py:1244`), `KNOWN_MODELS` and `MODEL_METADATA` (1239-1241) | here, in `catalog.rs`, as `ModelCatalog` |
//! | `_load_cached_model_discovery` (`agent.py:1267`), `_persist_model_discovery_cache` (1292) | here, in `cache.rs` |
//! | `lmstudio_models_root` (`agent.py:1312`) | the setting is `shed-config`'s `LmStudio`; the directory it resolves to is handed to the scan |
//! | `discover_lm_studio_models_from_disk` (`agent.py:1327`) | here, in `disk.rs` |
//! | `MODEL_PATHS` (`agent.py:1240`, written at 1258) | Stage 4, as `ModelCatalog::path_for`: its value is always `{name: name}` and its only readers (6835-6858) are inside `call_lm_engine` |
//! | `_apply_known_models`'s browser tail (`agent.py:1260-1264`) | Stage 3, with `BROWSER_MODEL_OPTIONS` and `BROWSER_DEFAULT_MODEL` |
//! | `resolve_AI_MODEL` (`agent.py:1727`) | Stage 3: every caller of it is a browser setting or a route (1262, 2180, 2193, 7973, 9201) and `resolve_background_model` (1761) is Stage 4, so it has no caller in Stage 2 and is not written here (`How_To_Code` principle 16) |
//! | `fetch_AI_MODEL_names` (`agent.py:1722`) | nowhere: it has no production caller (`docs/PARITY.md`, "Surfaces the clone does not carry") |
//! | `sort_browser_model_options`, `normalize_browser_*` (`utils/models.py:38-115`) | Stage 3, with the browser routes that call them |
//! | `resolve_model_context_window`, `format_model_*` (`agent.py:6408-6500`) | Stage 3, with the routes that report them |

mod cache;
mod catalog;
mod discovery;
mod disk;
mod entry;
mod size;

pub use catalog::{ModelCatalog, ModelMetadata};
pub use discovery::apply_discovered_payload;
