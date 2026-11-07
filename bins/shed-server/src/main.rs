//! The service binary (plan D9: Windows service `entity_in_the_shed`).
//!
//! Startup order clones `run_http_server` (`agent.py:11553–11592`): load and
//! validate configuration first, install logging, discover the models, print
//! the banner, serve. The schedulers the source starts between discovery and
//! the banner arrive with the stages that port what they run. Every startup
//! failure is one clean line naming the key or address at fault — Python
//! printed a traceback from deep inside import-time code.
//!
//! `--smoke` boots the real router on an ephemeral loopback port, fetches
//! `/health` over a raw HTTP/1.1 request (no HTTP client dependency),
//! verifies the response, and exits. The Stage 8 installer runs it before
//! the service is registered, so a broken `.env` or router never reaches the
//! service control manager.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use shed_config::AppConfig;
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
    let config = AppConfig::load().map_err(|error| error.to_string())?;
    shed_logging::install(config.log_level)?;
    // `--smoke` skips discovery on purpose: the installer's probe must pass on
    // a machine with LM Studio switched off, and `/health` answers without a
    // catalog. Everything else boots the way the source boots.
    if args.iter().any(|arg| arg == "--smoke") {
        return smoke(config).await;
    }
    discover_models(&config).await;
    serve(config).await
}

/// Model discovery: `run_http_server`'s `discover_lm_studio_models()`
/// (`agent.py:11559`), the step before the banner.
///
/// Best-effort like the source: a silent engine is not a failed boot, because
/// the fallback chain inside `shed_models::apply_discovered_payload` — cache
/// file, then disk scan — runs whatever the engine answered, and every branch
/// of it says in its own log line which way it took.
///
/// The catalog itself is dropped here, which is not waste but timing: no route
/// answers with a model name before Stage 3, so the catalog has no reader yet
/// (`How_To_Code` principle 16), and what survives this boot is the cache file
/// the discovery persisted — the same file the next boot's fallback chain
/// restores, exactly as it does in the source across a restart. The named
/// owner that carries the catalog across requests arrives with the routes.
async fn discover_models(config: &AppConfig) {
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
}

/// The banner and the listener. Python printed the banner before binding,
/// so a port clash crashed after the promise; the clone binds first and
/// only then prints what is now true.
async fn serve(config: AppConfig) -> Result<(), String> {
    let host = config.host.clone();
    let port = config.port;
    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|error| format!("cannot bind {host}:{port}: {error}"))?;
    println!("Starting The Entity In The Shed HTTP server on {host}:{port}");
    println!("  server=hyper (tokio)");
    let router = shed_http::build_router(Arc::new(config));
    axum::serve(listener, router)
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
    let router = shed_http::build_router(Arc::new(config));
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
