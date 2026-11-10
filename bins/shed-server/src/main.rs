//! The service binary (plan D9: Windows service `entity_in_the_shed`).
//!
//! Startup order clones `run_http_server` (`agent.py:11553–11592`): load and
//! validate configuration first, install logging, discover the models,
//! warm the character catalog, print the banner, serve. The schedulers the
//! source starts between discovery and the banner arrive with the stages
//! that port what they run. Every startup failure is one clean line naming
//! the key or address at fault — Python printed a traceback from deep
//! inside import-time code.
//!
//! `--smoke` boots the real router on an ephemeral loopback port, fetches
//! `/health` over a raw HTTP/1.1 request (no HTTP client dependency),
//! verifies the response, and exits. The Stage 8 installer runs it before
//! the service is registered, so a broken `.env` or router never reaches the
//! service control manager.
//!
//! `--base-dir <path>` names the directory the `.env` and the page's files
//! resolve against, for a dev build whose executable sits in `target/debug`
//! beside none of them. An option no one recognises stops the boot rather
//! than being walked past (principle 14).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use shed_config::AppConfig;
use shed_http::state::AppCore;
use shed_llm::{discover_lm_studio_models, new_request_id, LmTransport};
use shed_models::ModelCatalog;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("entity_in_the_shed: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(args: &[String]) -> Result<(), String> {
    let home = base_dir(args)?;
    let config = match &home {
        Some(dir) => AppConfig::load_from(dir),
        None => AppConfig::load(),
    }
    .map_err(|error| error.to_string())?;
    shed_logging::install(config.log_level)?;
    // `--smoke` skips discovery on purpose: the installer's probe must pass on
    // a machine with LM Studio switched off, and `/health` answers without a
    // catalog. Everything else boots the way the source boots.
    if args.iter().any(|arg| arg == "--smoke") {
        return smoke(config).await;
    }
    let catalog = discover_models(&config).await;
    serve(config, catalog).await
}

/// `--base-dir <path>`: the directory the configuration and the page's files
/// resolve against.
///
/// The installed service needs no flag — the Stage 8 installer puts `.env`,
/// `templates/`, `styles/`, `static/` and `characters/` beside the executable,
/// which is what [`AppConfig::load`] uses. A dev build's executable lives in
/// `target/debug`, beside none of them, so `cargo run` names the repository
/// instead. Absent the flag both forms are the same process.
///
/// A missing option value and a typo'd directory both stop the boot: falling
/// back to the defaults quietly would serve an empty page and an empty picker,
/// the failure a developer is least able to read.
fn base_dir(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut home: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--smoke" {
            index += 1;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (arg, None),
        };
        if name != "--base-dir" {
            return Err(format!("unknown option: {arg}"));
        }
        let value = match inline {
            Some(value) => value,
            None => {
                index += 1;
                match args.get(index) {
                    Some(value) => value.clone(),
                    None => return Err("--base-dir needs a directory".to_owned()),
                }
            }
        };
        if value.is_empty() {
            return Err("--base-dir needs a directory".to_owned());
        }
        let path = PathBuf::from(value);
        if !path.is_dir() {
            return Err(format!("--base-dir is not a directory: {}", path.display()));
        }
        home = Some(path);
        index += 1;
    }
    Ok(home)
}

/// Model discovery: `run_http_server`'s `discover_lm_studio_models()`
/// (`agent.py:11559`), the step before the banner.
///
/// Best-effort like the source: a silent engine is not a failed boot, because
/// the fallback chain inside `shed_models::apply_discovered_payload` — cache
/// file, then disk scan — runs whatever the engine answered, and every branch
/// of it says in its own log line which way it took.
///
/// The catalog is returned, not dropped: since Stage 3 the routes answer
/// with model names, so what discovery produced is handed to `AppCore` and
/// served, which is the `KNOWN_MODELS` global the source's discovery writes
/// and `/api/models` reads (`agent.py:1256`, `8550`).
async fn discover_models(config: &AppConfig) -> ModelCatalog {
    let mut catalog = ModelCatalog::default();
    let transport = LmTransport::new(&config.lm);
    discover_lm_studio_models(
        &transport,
        &mut catalog,
        &config.storage_root,
        config.lm_studio.models_dir.as_deref(),
        &new_request_id(),
    )
    .await;
    catalog
}

/// The banner and the listener. Python printed the banner before binding,
/// so a port clash crashed after the promise; the clone binds first and
/// only then prints what is now true.
async fn serve(config: AppConfig, catalog: ModelCatalog) -> Result<(), String> {
    let host = config.host.clone();
    let port = config.port;
    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|error| format!("cannot bind {host}:{port}: {error}"))?;

    let state = Arc::new(AppCore::new(config, catalog));
    // `start_character_catalog_warmer` (`agent.py:2704-2705`, called at
    // `11570`): a daemon thread between the schedulers and the banner, so
    // the first picker load does not pay for reading every country file.
    // `spawn_blocking` is the daemon thread — synchronous file I/O kept off
    // the async workers, and nobody joins it, exactly like a daemon.
    let warmer = Arc::clone(&state);
    tokio::task::spawn_blocking(move || warmer.warm_character_catalog());

    println!("Starting The Entity In The Shed HTTP server on {host}:{port}");
    println!("  server=hyper (tokio)");
    axum::serve(listener, shed_http::build_router(state))
        .with_graceful_shutdown(ctrl_c())
        .await
        .map_err(|error| format!("HTTP server stopped: {error}"))
}

/// Ctrl-C ends the run cleanly; the Windows service control path arrives
/// with the Stage 8 installer.
async fn ctrl_c() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("cannot listen for ctrl-c: {error}");
    }
}

/// Boot, probe, verify, exit. Deliberately independent of APP_HOST/APP_PORT:
/// the probe must work while the old Python service still owns port 5005.
async fn smoke(config: AppConfig) -> Result<(), String> {
    // The empty catalog is the honest state here: `--smoke` skips discovery,
    // and `/health` reads only configuration, so the probe passes whatever
    // catalog the engine would have produced — including none.
    let state = Arc::new(AppCore::new(config, ModelCatalog::default()));
    let router = shed_http::build_router(state);
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("smoke: cannot bind an ephemeral port: {error}"))?;
    let addr = listener
        .local_addr()
        .map_err(|error| format!("smoke: {error}"))?
        .to_string();
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("smoke: server error: {error}");
        }
    });
    let response = fetch_health(&addr).await?;
    let split = response
        .find("\r\n\r\n")
        .ok_or_else(|| format!("smoke: response has no header terminator: {response:?}"))?;
    let (head, body) = response.split_at(split + "\r\n\r\n".len());
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!(
            "smoke: unexpected status line: {}",
            head.lines().next().unwrap_or("")
        ));
    }
    if !head.contains("application/json") {
        return Err(format!("smoke: missing json content-type in: {head}"));
    }
    if !body.ends_with("}\n") || !body.contains("\"status\":\"ok\"") {
        return Err(format!("smoke: unexpected body: {body:?}"));
    }
    println!("smoke: /health ok ({} body bytes)", body.len());
    Ok(())
}

/// A raw HTTP/1.1 GET — the same bytes a browser's request line starts
/// with, with `Connection: close` so hyper ends the stream and the read
/// terminates (the U3 check on hyper's shutdown behaviour).
async fn fetch_health(addr: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|error| format!("smoke: cannot connect to {addr}: {error}"))?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: smoke\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|error| format!("smoke: request failed: {error}"))?;
    let mut text = String::new();
    stream
        .read_to_string(&mut text)
        .await
        .map_err(|error| format!("smoke: response read failed: {error}"))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::base_dir;

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn no_flag_leaves_the_home_to_the_executable() {
        assert!(
            matches!(base_dir(&owned(&["--smoke"])), Ok(None)),
            "smoke alone boots exactly as the installed service does"
        );
    }

    #[test]
    fn both_spellings_name_the_directory() {
        let temp = std::env::temp_dir();
        let text = temp.to_string_lossy().into_owned();
        for spelling in [
            owned(&["--base-dir", text.as_str()]),
            owned(&[&format!("--base-dir={text}")]),
        ] {
            match base_dir(&spelling) {
                Ok(Some(found)) => assert_eq!(found, temp),
                other => panic!("the flag must name the directory: {other:?}"),
            }
        }
    }

    #[test]
    fn smoke_and_base_dir_combine_in_either_order() {
        let text = std::env::temp_dir().to_string_lossy().into_owned();
        for spelling in [
            owned(&["--smoke", "--base-dir", text.as_str()]),
            owned(&["--base-dir", text.as_str(), "--smoke"]),
        ] {
            assert!(base_dir(&spelling).is_ok(), "{spelling:?} must parse");
        }
    }

    #[test]
    fn an_unknown_option_stops_the_boot() {
        let error = match base_dir(&owned(&["--smok"])) {
            Err(error) => error,
            Ok(value) => panic!("a typo'd flag must not boot the service: {value:?}"),
        };
        assert!(error.contains("unknown option"), "{error}");
    }

    #[test]
    fn a_home_that_is_not_a_directory_names_the_problem() {
        for (spelling, wanted) in [
            (owned(&["--base-dir"]), "needs a directory"),
            (owned(&["--base-dir="]), "needs a directory"),
            (
                owned(&["--base-dir", "Z:\\definitely-not-the-repository"]),
                "not a directory",
            ),
        ] {
            let error = match base_dir(&spelling) {
                Err(error) => error,
                Ok(value) => panic!("{} must fail: {value:?}", spelling.join(" ")),
            };
            assert!(error.contains(wanted), "{error}");
        }
    }
}
