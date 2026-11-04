//! `--smoke`: boot the real configuration, answer one request to itself, and exit.
//!
//! The stage gate has to prove the app is bootable without taking the port the operator's own
//! app is already answering on, so this binds `127.0.0.1:0` and lets the system pick. The
//! configured address is still reported, because that is the address the operator cares about.
//!
//! The request goes over a real socket rather than through the router in memory, which
//! `crates/shed-http/src/health.rs` already does. A socket is where a response gets finished:
//! `Content-Length` is written by the HTTP layer as it serves, so it is a byte an in-memory
//! call never sees.
//!
//! The request asks for `Connection: close`, which is what ends a response by closing the
//! connection. Without it the connection would stay open and the reader would have no point at
//! which it could say it had the whole answer. The client does not half-close its own side
//! instead: hyper's `half_close` option is off by default, so a `FIN` that arrives while the
//! request is still being read closes the connection before the request is dispatched
//! (`hyper-1.12.0/src/server/conn/http1.rs:260`), and the smoke run would read nothing back.

use std::error::Error;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use shed_config::AppConfig;
use shed_http::{AppState, router};

use crate::failure;

/// The whole request: `/health` takes no parameters, no body, and no authentication.
const REQUEST: &[u8] = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";

/// The status line that says the route answered.
const OK: &str = "HTTP/1.1 200 OK";

/// The content type `jsonify` writes.
const JSON: &str = "application/json";

/// How long the process gives itself to answer its own request before it calls that a failure.
/// The answer needs no model and no disk work, so anything this slow is a hang, not a slow turn.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// Boot, answer, report, exit.
pub async fn run(config: &AppConfig) -> ExitCode {
    let (shutdown, wake) = tokio::sync::oneshot::channel::<()>();

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(source) => {
            return failure(&format!("no temporary port was available to answer on: {source}"), 1);
        }
    };

    let server = axum::serve(listener, router(AppState::new(config.clone())))
        .with_graceful_shutdown(async move {
            let _ = wake.await;
        });

    let port = match server.local_addr() {
        Ok(address) => address.port(),
        Err(source) => return failure(&format!("the temporary port: {source}"), 1),
    };

    // `Serve` and `WithGracefulShutdown` are `IntoFuture` rather than `Future`, so the server
    // is spawned as an async block that awaits it.
    let serving = tokio::spawn(async move { server.await });

    let answer = tokio::time::timeout(ANSWER_TIMEOUT, answer_our_health(port)).await;

    // Whatever the answer was, this run is over: the signal closes the listener, and waiting
    // for the server here is what keeps a finished smoke run from leaving one behind.
    let _ = shutdown.send(());
    let stopped = match serving.await {
        Ok(result) => result,
        Err(panicked) => return failure(&format!("the server task ended: {panicked}"), 1),
    };

    let address = format!("{}:{}", config.server.host, config.server.port);

    let summary = match answer {
        Ok(Ok(summary)) => summary,
        Ok(Err(problem)) => {
            return failure(&format!("GET /health on {address} answered badly: {problem}"), 1);
        }
        Err(_) => {
            let secs = ANSWER_TIMEOUT.as_secs();
            return failure(&format!("{address} did not answer its own /health in {secs}s"), 1);
        }
    };

    match stopped {
        Ok(()) => {
            tracing::info!("smoke: GET /health on {address} answered {OK} with {summary}");
            ExitCode::SUCCESS
        }
        Err(source) => failure(&format!("the server stopped before it should have: {source}"), 1),
    }
}

/// Ask for `GET /health` over a socket and check the bytes that come back.
///
/// The checks are the ones the captured Python response pins: the status line, the content
/// type, a `Content-Length` that agrees with the body it counts, and a body that says `ok`.
async fn answer_our_health(port: u16) -> Result<String, Box<dyn Error + Send + Sync>> {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await?;
    socket.write_all(REQUEST).await?;

    // `Connection: close` means the end of the response is the end of the connection, so
    // reading to the end reads exactly one response.
    let mut response = Vec::new();
    socket.read_to_end(&mut response).await?;

    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("no end of headers in the response")?;

    let mut lines = head.lines();
    let status = lines.next().ok_or("no status line in the response")?;
    if status != OK {
        return Err(format!("status line was {status:?}, not {OK:?}").into());
    }

    let mut content_type: Option<&str> = None;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("header line was not name and value: {line:?}").into());
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-type" => content_type = Some(value),
            "content-length" => content_length = Some(value.parse::<usize>()?),
            _ => {}
        }
    }

    if content_type != Some(JSON) {
        return Err(format!("content type was {content_type:?}, not {JSON:?}").into());
    }

    // The point of doing this over a socket: the length is the HTTP layer's own count of the
    // body, and it counts the newline `jsonify` writes.
    if content_length != Some(body.len()) {
        return Err(format!(
            "content length was {content_length:?} for a body of {} bytes",
            body.len()
        )
        .into());
    }

    let payload: Value = serde_json::from_str(body)?;
    if payload["status"] != "ok" {
        return Err(format!("the payload did not say ok: {payload}").into());
    }

    Ok(format!("{} bytes of JSON", body.len()))
}
