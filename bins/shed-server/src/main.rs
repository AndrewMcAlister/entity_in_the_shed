//! The process: read the settings, install the log, answer HTTP.
//!
//! Ported from `agent.py:11553` (`run_http_server`) and `agent.py:11606` (the entry point).
//!
//! The source does five things before it serves: it discovers the models the engine is
//! holding, starts the retention scheduler, the conversation housekeeper, the character
//! catalogue warmer, and the entity's own clock. Each of those arrives with the stage that
//! ports the code it runs, so Stage 1 starts the two things Stage 1 has: the log and the
//! server. `initialize_mctrader_credentials` (`agent.py:11519`) is a `pass` in the source and
//! is not ported at all.
//!
//! `--smoke` is not in the source. It is this workspace's own gate: the same settings, the
//! same log, the same router, on a port nobody else is holding, one request answered to
//! itself, and an exit code that says whether the app is bootable. See `smoke.rs`.

mod smoke;

use std::process::ExitCode;

use shed_config::AppConfig;
use shed_http::{AppState, router};
use tokio::net::TcpListener;

/// What the process was started to do.
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// Answer HTTP on the configured address until the process is stopped.
    Serve,
    /// Boot the same way, answer one request to itself, and exit.
    Smoke,
}

impl Mode {
    /// `--smoke` is the only argument this process takes, which is more than the source takes:
    /// `python agent.py` starts the server and reads nothing off the command line. An argument
    /// nobody recognised is a mistake worth naming rather than a default to fall into.
    fn from_args<I: Iterator<Item = String>>(args: I) -> Result<Self, String> {
        let args: Vec<String> = args.collect();
        match args.first().map(String::as_str) {
            None => Ok(Self::Serve),
            Some("--smoke") if args.len() == 1 => Ok(Self::Smoke),
            _ => Err(format!("unrecognised argument(s): {}", args.join(" "))),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let mode = match Mode::from_args(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(message) => return failure(&message, 2),
    };

    // Reading the settings is also what creates the storage root, which is what the source does
    // at import time. Settings that cannot be read are not a server worth starting.
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => return failure(&error.to_string(), 1),
    };

    shed_logging::init(config.log_level);

    match mode {
        Mode::Serve => serve(&config).await,
        Mode::Smoke => smoke::run(&config).await,
    }
}

/// Say what went wrong on stderr, and name the exit code.
///
/// The log is not used here because both failures it reports happen around the log rather than
/// in it: settings that could not be read (before the log exists) and an address that will not
/// answer (after the log has nothing else to say).
pub(crate) fn failure(message: &str, code: u8) -> ExitCode {
    eprintln!("shed-server: {message}");
    ExitCode::from(code)
}

/// The line `agent.py:11577` prints, printed the way it is printed there: to stdout, through
/// `print`, before the socket is opened.
///
/// `print_stdout` is a workspace warning because app narration belongs in the log. This is not
/// app narration: it is the process announcing itself to the terminal that started it, which is
/// what the source does, and a log line would be a second copy of the same sentence. The
/// `  server=werkzeug (threaded)` line the source adds next is not ported, because
/// `USE_WAITRESS` is dropped (decision D5) and there is no framework to name.
fn announce(config: &AppConfig) {
    // The allow is on a block rather than on the `println!` line: an attribute applied to a
    // macro invocation is ignored by the compiler, so it would be an error of its own.
    #[allow(clippy::print_stdout)]
    {
        println!(
            "Starting The Entity In The Shed HTTP server on {}:{}",
            config.server.host, config.server.port
        );
    }
}

/// Bind, say so, and answer until the process is stopped.
async fn serve(config: &AppConfig) -> ExitCode {
    announce(config);

    let host = config.server.bind_host();
    let port = config.server.port;
    let listener = match TcpListener::bind((host, port)).await {
        Ok(listener) => listener,
        Err(source) => return failure(&format!("{host}:{port} could not be bound: {source}"), 1),
    };

    match axum::serve(listener, router(AppState::new(config.clone()))).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(source) => failure(&format!("the server stopped: {source}"), 1),
    }
}
