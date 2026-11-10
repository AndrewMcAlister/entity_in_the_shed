//! The state every route shares: one per process, behind an `Arc`.
//!
//! The Python app keeps this in module globals — `KNOWN_MODELS`,
//! `MODEL_METADATA`, the character caches, `TTS_PROVIDER` — written at import
//! time and read by whichever view needs it. Here it is one value built at
//! startup and handed to the router, which is what `agent.py:1256`'s
//! `global KNOWN_MODELS, MODEL_PATHS, MODEL_METADATA` was reaching for: the
//! discovered catalog has to outlive the function that discovered it and stay
//! visible to the routes, and an `Arc` says that out loud where a global only
//! implies it.
//!
//! The caches belong to the catalogs that own them
//! (`CharacterCatalog::index`/`files` are `agent.py:942`/`948`), not to this
//! struct, so a route never has to remember to invalidate anything.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use shed_characters::catalog::{characters_dir, CharacterCatalog};
use shed_config::AppConfig;
use shed_models::{ContextSettings, ModelCatalog};

use crate::page::PagePaths;

/// Everything a handler reads. Built once, by the binary, after the model
/// discovery and before the first request.
pub struct AppCore {
    /// The startup configuration; read-only after `AppConfig::from_source`.
    pub config: AppConfig,
    /// `KNOWN_MODELS` + `MODEL_METADATA` (`agent.py:1239`/`1241`). Behind a
    /// lock because `/api/refresh-models` (`agent.py:11595-11603`) rewrites
    /// them in place while the process runs — the one piece of the source's
    /// module state that is not fixed at boot — and every other route reads
    /// it. The lock is only ever held for a synchronous read or a swap,
    /// never across an await.
    models: Mutex<ModelCatalog>,
    /// The four context-window settings, read once instead of per call.
    pub context: ContextSettings,
    /// The country index and per-country file caches (`agent.py:942`–`948`).
    pub characters: CharacterCatalog,
    /// The page and asset files the browser asks for by name.
    pub paths: PagePaths,
    /// `index.html` with its three values substituted, rendered on the first
    /// request for it and reused after that. The page is a static file whose
    /// substitutions come from settings that cannot change while the process
    /// runs, so re-rendering per request — what Jinja does — would read 189 KB
    /// from disk and rescan it for every keystroke of a conversation.
    rendered_page: OnceLock<Vec<u8>>,
}

/// The handle axum clones into each request.
pub type AppState = Arc<AppCore>;

impl AppCore {
    /// Assemble the state from the config and the catalog discovery produced.
    ///
    /// The catalog arrives rather than being discovered here: `shed-http`
    /// serves whatever the engine stage found, and a router that could swap
    /// the catalog mid-flight would be a race the Python app has and this one
    /// does not need.
    #[must_use]
    pub fn new(config: AppConfig, models: ModelCatalog) -> Self {
        let context = ContextSettings::from_config(&config);
        let paths = PagePaths::new(&config.base_dir);
        let characters = CharacterCatalog::new(characters_dir(&config.base_dir));
        Self {
            config,
            models: Mutex::new(models),
            context,
            characters,
            paths,
            rendered_page: OnceLock::new(),
        }
    }

    /// Read the catalog: the closure runs under the lock, so the borrow it
    /// gets is the real `&ModelCatalog` rather than a clone of it, and the
    /// lock is gone the moment the closure returns.
    pub fn with_models<T>(&self, read: impl FnOnce(&ModelCatalog) -> T) -> T {
        read(&self.lock_models())
    }

    /// Replace the catalog wholesale — `global KNOWN_MODELS` after a
    /// discovery (`agent.py:1256`), which is what a refresh does.
    pub fn replace_models(&self, next: ModelCatalog) {
        *self.lock_models() = next;
    }

    /// The catalog's lock. Poisoning would mean a panic happened while a
    /// route held it, and `ModelCatalog` has no invariant a panic could
    /// leave broken in it — the value under the lock is still consistent,
    /// so the guard is taken anyway rather than failing the request that
    /// merely wanted to read it.
    fn lock_models(&self) -> MutexGuard<'_, ModelCatalog> {
        self.models
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The rendered page when it is already in hand.
    #[must_use]
    pub fn cached_page(&self) -> Option<&[u8]> {
        self.rendered_page.get().map(Vec::as_slice)
    }

    /// Store the rendered page. The first writer wins; a later call keeps the
    /// bytes already there, which is what a cache is for.
    pub fn store_page(&self, rendered: Vec<u8>) -> &[u8] {
        self.rendered_page.get_or_init(|| rendered).as_slice()
    }

    /// `warm_character_catalog` (`agent.py:2690-2701`): populate the country
    /// index and the voice catalog off the request path, so the first picker
    /// load does not pay for the first read of every `characters/` file.
    ///
    /// The source starts this on a daemon thread before the banner
    /// (`agent.py:2704-2705`, called at `11570`); the equivalent here is
    /// `tokio::task::spawn_blocking` from the binary, because this is
    /// synchronous file I/O and the request threads should not pay for it.
    /// The source's `except` arm logs a warning; neither `country_list` nor
    /// `list_available_voices` has a failure left to take — both return the
    /// empty value and log their own cause — so the ready line is the only
    /// outcome, and the count in it is the diagnostic the `except` was
    /// protecting.
    pub fn warm_character_catalog(&self) {
        let countries = self.characters.country_list();
        let voices = shed_tts::list_available_voices(&self.config.tts.provider, "");
        tracing::info!(
            "Character catalog ready: {} countries, {} voices available",
            countries.len(),
            voices.len(),
        );
    }
}
