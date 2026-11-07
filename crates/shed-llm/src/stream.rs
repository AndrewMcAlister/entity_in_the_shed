//! The streamed chat completion: an OpenAI-style SSE body read token by token, and the retry
//! ladder in front of it that stops retrying the moment anything has been spoken.
//!
//! Ported from `agent.py:1073` (`LmStreamUnavailable`), `1081` (`read_lm_chat_stream`), `1153`
//! (`call_lm_http_stream_chat`), `1213` (`_stream_rejection`) and `1223` (`_lm_stream_backoff`).
//!
//! ## Why the ladder here is not `retry.rs`'s
//!
//! `retry.rs`'s ladder can replay a request freely: nothing has been shown to anyone while it
//! retries. A streamed reply is different — once a delta has reached the caller it cannot be
//! unsaid, and replaying the request would speak the reply from its first word again. So this
//! ladder tracks, per attempt, whether the attempt delivered anything (`self_delivered` at 1168),
//! and a failure after that point raises instead of waiting.
//!
//! ## The three ways this ladder fails, and why they are three
//!
//! - [`LmStreamUnavailable`]: the server refused to stream at all, while nothing had been spoken.
//!   The caller may repeat the turn down the non-streaming path, and does (7016).
//! - [`StreamFailure`]: a mid-stream failure raised raw, exactly as 1202 re-raises the source
//!   exception. It says *this turn broke partway*; it must not be dressed up as the ladder's
//!   `RuntimeError`, because the ladder did not run it out.
//! - [`LmRequestError`]: a failure the source wraps in `RuntimeError(f"LM request failed during
//!   {purpose}: {exc}")` — the POST that never produced a head (1182) and the ladder running out
//!   (1210). Both carry the same text, so a caller sees one shape for "the request failed".
//!
//! ## Divergences, recorded in `docs/PARITY.md`, "Structural changes"
//!
//! The source's `on_delta` handler is wrapped in `try/except` (1143-1148) so a failing progress
//! sink cannot cost the user the reply; a `Fn` closure cannot fail, so the clone has nothing to
//! catch. And `requests`' `(idle, idle)` socket timeout restarts on every chunk, while reqwest
//! applies a per-request read timeout to the response head — the clone's real stall guard is the
//! deadline, checked per line, which is what the source relies on too (1097).

use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::{Error as ReqwestError, Response};
use serde_json::Value;
use tokio::time::{sleep, timeout, Instant};
use tracing::{debug, warn};

use shed_text::safe_body_snippet;

use crate::request_id::new_request_id;
use crate::retry::{
    backoff_ms, is_transient_exception, is_transient_status, LmRequestError, LmTransport,
};

/// The sink that hears the reply as it is written.
///
/// Stage 4 hands this to the route that relays deltas to the browser; the parity suite is its
/// caller today. It takes `&str` rather than an owned `String` because the source passes the
/// decoded piece itself, and a caller that wants to keep a piece copies it.
pub type DeltaSink<'a> = &'a (dyn Fn(&str) + Sync);

/// The server refused a streamed completion before delivering anything.
///
/// Raised only while nothing has been spoken, so the caller can safely repeat the turn down the
/// non-streaming path — which is exactly what 7016 does with it.
#[derive(Debug)]
pub struct LmStreamUnavailable {
    detail: String,
}

impl LmStreamUnavailable {
    /// Wrap a rejection description — the text [`rejection_text`] produced, or anything else that
    /// says why the server would not stream.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for LmStreamUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl StdError for LmStreamUnavailable {}

/// The whole of a streamed completion, shaped exactly like the JSON body the non-streaming path
/// parses (1084-1085): the content, the finish reason, and the usage, so a caller cannot tell the
/// two paths apart. The Stage-4 caller maps `usage`'s `completion_tokens` and `prompt_tokens`
/// onto `eval_count` and `prompt_eval_count` (6632-6635), exactly as it does for a non-streamed
/// body, because this is the same shape.
///
/// `pieces` has no Python counterpart because `str.join` has nothing to keep: it is what lets a
/// test assert *which* deltas were spoken and in what order, which is the one thing the streaming
/// path exists to provide.
#[derive(Debug)]
pub struct StreamRead {
    pub content: String,
    pub finish_reason: String,
    pub usage: Value,
    pub pieces: Vec<String>,
}

impl Default for StreamRead {
    fn default() -> Self {
        Self {
            content: String::new(),
            finish_reason: String::new(),
            // `usage = {}` (1090): the source's default is an empty object, not a null, because
            // the caller reads `usage.get("completion_tokens")` off it either way.
            usage: Value::Object(serde_json::Map::new()),
            pieces: Vec::new(),
        }
    }
}

/// `raise RuntimeError("Model generation timed out while streaming.")` (1098), as a type.
///
/// The message is verbatim because it reaches a user: `api_error_response(str(exc), …)` on the
/// routes that wrap it.
#[derive(Debug)]
pub struct StreamTimeout;

impl fmt::Display for StreamTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Model generation timed out while streaming.")
    }
}

impl StdError for StreamTimeout {}

/// A streamed completion that broke before its end — and, unlike a rejection, broke *after* the
/// status was already a success, so the only question left is whether anything was spoken.
#[derive(Debug)]
pub enum StreamFailure {
    /// The deadline passed while the body was still open (1097-1098). Never retried: the source's
    /// `RuntimeError` is neither of `_is_retryable_lm_failure`'s two exception classes.
    Timeout(StreamTimeout),
    /// Reading the body raised — a dropped connection, a stalled socket, a chunked frame that
    /// ended mid-line. Boxed rather than `std::io::Error` because the same variant also carries
    /// the reqwest errors the transport surfaces from `chunk()`.
    Transport(Box<dyn StdError + Send + Sync + 'static>),
}

impl StreamFailure {
    /// Wrap a transport failure: the reqwest error from `chunk()`, or the `std::io::Error` from a
    /// raw-socket test server standing in for a dropped engine.
    pub(crate) fn transport(error: impl StdError + Send + Sync + 'static) -> Self {
        Self::Transport(Box::new(error))
    }
}

impl fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout(timeout) => timeout.fmt(f),
            Self::Transport(error) => error.fmt(f),
        }
    }
}

impl StdError for StreamFailure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Timeout(timeout) => Some(timeout),
            Self::Transport(error) => Some(&**error),
        }
    }
}

/// What the ladder raises with: one of the three shapes the module docs describe.
///
/// A boxed trait object rather than a crate enum because two of the three are the crate's own
/// errors and the third — the raw mid-stream raise — is whatever the body read surfaced, which in
/// the source is `iter_lines`' own exception and in the clone is a `StreamFailure` wrapping a
/// reqwest error or the `io::Error` of a socket that died. A caller that cares matches on
/// [`LmStreamUnavailable`] to fall back; everything else is reported as it reads.
type LadderError = Box<dyn StdError + Send + Sync + 'static>;

/// `call_lm_http_stream_chat` (`agent.py:1153`): POST a streamed chat completion, retrying only
/// before anything is spoken.
///
/// `payload` is the whole request body the caller assembled — the corridor's one dynamic value,
/// which this crate passes through without reading (see the module docs in `lib.rs`). The source's
/// caller sets `body["stream"] = True` (6604) before arriving here, so this function does not add
/// it: what the body says is the caller's, exactly as in the source.
///
/// `idle_timeout` is the source's `timeout=(idle, idle)` tuple (1176, assembled at 6606-6621);
/// `deadline` is its `time.monotonic() + reply_timeout` (6611), the check that actually guards a
/// stalled stream — see the divergence note in the module docs.
///
/// `Err` carrying `LmStreamUnavailable` means the caller may repeat the turn without streaming;
/// a `StreamFailure` means the turn broke partway and must not be repeated; an `LmRequestError`
/// is the source's wrapper, for a request that never produced a head.
pub async fn call_lm_http_stream_chat(
    transport: &LmTransport,
    purpose: &str,
    payload: &Value,
    idle_timeout: Option<Duration>,
    deadline: Option<Instant>,
    on_delta: DeltaSink<'_>,
) -> Result<StreamRead, LadderError> {
    let call = Call {
        transport,
        purpose,
        payload,
        idle_timeout,
        deadline,
        on_delta,
    };
    let request_id = new_request_id();
    let retries = call.retries();
    let attempts = retries + 1; // 1226 prints `LM_ENGINE_TRANSIENT_RETRIES + 1`: attempts, not retries.

    // The source's loop is `while attempt <= retries` with an `attempt > retries` guard *inside*
    // it (1201). The guard is what makes the final attempt raise the raw exception rather than
    // wait for another try, so the clone keeps the same shape rather than folding the guard into
    // the loop condition and quietly changing which error the last attempt raises.
    for attempt in 1..=attempts {
        match attempt_stream(&call, &request_id, attempt, retries).await {
            Outcome::Read(read) => return Ok(read),
            Outcome::Backoff(detail) => {
                lm_stream_backoff(&call, &request_id, attempt, &detail, attempts).await;
            }
            Outcome::Raise(error) => return Err(error),
        }
    }

    unreachable!(
        "the stream ladder always leaves its loop through a return or a raise: every attempt ends \
         in a delta-read, a rejection, a raise, or a `continue` that requires `attempt <= retries` \
         (agent.py:1179, 1185, 1201), so the loop is re-entered — which is why the source's own \
         1210 is as unreachable as 1070's, and why `last_error` is never read here either"
    )
}

/// One attempt of the ladder: 1166-1208, from POST to `finally`.
enum Outcome {
    Read(StreamRead),
    /// Retry with the source's wait, after the warning line; `detail` is the `{exc}` or the
    /// `status=…` that line names (1180, 1186, 1203).
    Backoff(String),
    /// Raise, and stop.
    Raise(LadderError),
}

/// The facts every attempt of the ladder reads unchanged: the parameters 1166-1208 reads on each
/// pass without touching.
///
/// Grouped because eight arguments handed to one attempt reads like a telegraph form, and because
/// it puts `retries` with the ladder's own settings rather than in the middle of a call. The
/// `Sync` bound the delta sink carries comes along, which is what keeps the whole ladder's future
/// `Send` for the served request that calls it at Stage 4.
struct Call<'a> {
    transport: &'a LmTransport,
    purpose: &'a str,
    payload: &'a Value,
    idle_timeout: Option<Duration>,
    deadline: Option<Instant>,
    on_delta: DeltaSink<'a>,
}

impl Call<'_> {
    /// `LM_ENGINE_TRANSIENT_RETRIES` (`agent.py:491`, clamped at zero where it is read).
    fn retries(&self) -> u64 {
        self.transport.engine().transient_retries
    }
}

/// One attempt: 1166-1208, from POST to `finally`.
async fn attempt_stream(call: &Call<'_>, request_id: &str, attempt: u64, retries: u64) -> Outcome {
    // `self_delivered` (1168): the source counts the pieces in a list; the clone counts the same
    // fact as a flag, because the only question ever asked of it is whether the list is empty.
    // `AtomicBool` rather than `Cell` because the counting closure is handed to the reader as the
    // `Sync` delta sink — and a `Sync` sink is what keeps the reader's future `Send`, which the
    // served request that calls this at Stage 4 needs.
    let delivered = AtomicBool::new(false);
    let counting = |piece: &str| {
        delivered.store(true, Ordering::Relaxed);
        (call.on_delta)(piece);
    };

    let mut response = match send_stream_head(call.transport, call.payload, call.idle_timeout).await
    {
        Ok(response) => response,
        Err(failure) => {
            // 1177-1182: retryable *and* attempts left, or the wrapper raises. The attempts half
            // is what makes the last attempt raise rather than sleep. Unlike the read failure
            // below, this one is always wrapped — 1182 builds the `RuntimeError` itself, because
            // nothing was ever delivered on an attempt that never got a response head.
            if is_transient_stream_failure(&failure) && attempt <= retries {
                return Outcome::Backoff(failure.to_string());
            }
            return Outcome::Raise(Box::new(LmRequestError::new(call.purpose, failure)));
        }
    };

    // 1185-1187 come before the rejection check: a 503 is retried, and only a non-transient
    // failure at or above 400 is a refusal to stream.
    let status = response.status().as_u16();
    if is_transient_status(status) && attempt <= retries {
        return Outcome::Backoff(format!("status={status}"));
    }

    if status >= 400 {
        // 1189-1193: nothing was delivered, so the caller is free to fall back to the
        // non-streaming path — which is also how an older server that rejects `stream` is handled.
        // The raise escapes the attempt the way 1193's escapes 1197's `except Exception`: a
        // refusal is not a failure to retry, it is a different answer.
        let detail = rejection_text(response).await;
        return Outcome::Raise(Box::new(LmStreamUnavailable::new(detail)));
    }

    match read_lm_chat_stream(
        &mut response,
        request_id,
        &counting,
        call.deadline,
        call.idle_timeout,
    )
    .await
    {
        Ok(read) => Outcome::Read(read),
        Err(failure) => {
            // 1197-1204, in the source's order: anything already delivered cannot be unsaid, so
            // a retry here would repeat it; then the exception's own retryability; then the
            // attempts left. The third is the guard 1201 writes and 1050 does not: on the final
            // attempt a clean-start failure is raised *raw* rather than wrapped, because the
            // wrapper at 1210 sits after the loop and this attempt is leaving through 1202.
            if delivered.load(Ordering::Relaxed)
                || !is_transient_stream_failure(&failure)
                || attempt > retries
            {
                return Outcome::Raise(Box::new(failure));
            }
            Outcome::Backoff(failure.to_string())
        }
    }
}

/// `requests.post(url, json=payload, timeout=timeout, stream=True)` (1176): the POST whose
/// response head arrives before the body has.
///
/// The failure is a [`StreamFailure`] rather than the transport's own error because the wait for a
/// head can expire twice over: reqwest's error for a socket that died, and the clone's own
/// `read_timeout_failure` for a window that ran out while the engine never answered.
///
/// `idle` is the source's read timeout, which in the tuple of 1176 covers the wait for the head as
/// much as the body — an engine that accepts a request and then thinks forever dies there, on
/// `ConnectTimeout`/`ReadTimeout`, both of which 1179 reads as transient. Neither of reqwest's
/// knobs belongs on the request: `timeout` is a *total* for the whole body, and a stream that
/// speaks slowly for longer than one idle window is legitimate. The clone therefore applies the
/// window to this wait alone, and the reader applies it again per chunk (see `pull_line`). The
/// total guard is the caller's deadline, and the production caller always sets one (6611).
async fn send_stream_head(
    transport: &LmTransport,
    payload: &Value,
    idle: Option<Duration>,
) -> Result<Response, StreamFailure> {
    let url = format!("{}/v1/chat/completions", transport.host());
    let request = transport.client().post(&url).json(payload);
    match idle {
        Some(window) => timeout(window, request.send())
            .await
            .map_err(|_| read_timeout_failure(window))?
            .map_err(StreamFailure::transport),
        None => request.send().await.map_err(StreamFailure::transport),
    }
}

/// `_lm_stream_backoff` (`agent.py:1223`): the warning line, then the wait it names.
///
/// The line is not the non-streaming ladder's line: the source's sentence here is
/// `LM transient failure during …` with the detail always appended, where 1053/1062 distinguish
/// an exception from a status. The wait itself is the same `backoff_ms`, as it is in the source —
/// 1224 repeats 1051's expression verbatim.
async fn lm_stream_backoff(
    call: &Call<'_>,
    request_id: &str,
    attempt: u64,
    detail: &str,
    attempts: u64,
) {
    let engine = call.transport.engine();
    let purpose = call.purpose;
    let wait = backoff_ms(engine, attempt);
    warn!(
        "[{request_id}] LM transient failure during {purpose} attempt={attempt}/{attempts}; \
         retrying in {wait}ms: {detail}"
    );
    sleep(Duration::from_millis(wait)).await;
}

/// `_stream_rejection` (`agent.py:1213`): describe a non-success streaming response the way the
/// JSON path does.
///
/// `status 404 with content_type application/json: no route` — the shape a log reader already
/// knows from `parse_json_response_strict`'s messages, because the source deliberately borrowed
/// it (1214's docstring says so). A body that cannot be read says `<could not read response body>`
/// (1216-1218), and a missing `Content-Type` says the empty string rather than `unknown` — the
/// two JSON-path messages differ in that detail and so do these (see `shed-json`'s `JsonError`).
///
/// Takes the response by value because reqwest's `text` does: reading a streamed body to the end
/// consumes the response, which is what 1216's `response.text` does to a `stream=True` response
/// too. The status and the headers are read before the body, because the body takes the response
/// with it; the source reads them after, and reads the same two values either way.
async fn rejection_text(response: Response) -> String {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => "<could not read response body>".to_string(),
    };
    format!(
        "status {status} with content_type {content_type}: {}",
        safe_body_snippet(&body)
    )
}

/// `_is_retryable_lm_failure` as the *streaming* ladder applies it (1179, 1185, 1201).
///
/// The set is 1032's, unchanged: `Timeout` and `ConnectionError`. What the clone reads as each of
/// them depends on where the failure came from:
///
/// - a POST that never got a head: reqwest's `is_connect` (the source's `ConnectionError`) or
///   `is_timeout` (its `ConnectTimeout`, which subclasses both classes);
/// - a body read that expired its idle window: the clone's own `read_timeout_failure`, an
///   `io::Error` of kind `TimedOut`, standing in for the source's `ReadTimeout`;
/// - a body that broke for any other reason — the engine died mid-stream, a chunked frame ended
///   early — is *not* in the set. `requests` reports that as `ChunkedEncodingError`, a subclass of
///   `IOError` and neither of 1032's classes, so the source re-raises it at 1201 whatever the
///   attempt count. reqwest surfaces the same break as a `Kind::Decode`/`Kind::Body` error, which
///   is equally outside the set here.
///
/// The non-streaming ladder never sees this distinction: it reads its body outside the ladder.
fn is_transient_stream_failure(failure: &StreamFailure) -> bool {
    match failure {
        // The source's `RuntimeError` from the deadline check is not a `Timeout` *exception* —
        // it is a fresh error the loop itself raises, and 1201 re-raises it untouched.
        StreamFailure::Timeout(_) => false,
        StreamFailure::Transport(error) => match error.downcast_ref::<ReqwestError>() {
            Some(error) => is_transient_exception(error),
            None => matches!(
                error.downcast_ref::<io::Error>().map(io::Error::kind),
                Some(io::ErrorKind::TimedOut)
            ),
        },
    }
}

/// `read_lm_chat_stream` (`agent.py:1081`): read an OpenAI-style SSE body, handing each content
/// delta on.
///
/// `deadline` is checked before each line is processed, as it is at 1097 — including before the
/// final line the buffer yields when the body ends, which the source's generator checks too. A
/// `None` deadline is the source's `deadline=None`: a caller with no configured client timeout.
pub async fn read_lm_chat_stream(
    response: &mut Response,
    request_id: &str,
    on_delta: DeltaSink<'_>,
    deadline: Option<Instant>,
    idle: Option<Duration>,
) -> Result<StreamRead, StreamFailure> {
    let mut read = StreamRead::default();
    let mut buffer: Vec<u8> = Vec::new();

    loop {
        // `iter_lines(chunk_size=512, decode_unicode=False)` (1096) splits on newlines across
        // chunk boundaries; the chunk size is a read hint, so the clone reads whole chunks as
        // `chunk()` hands them and splits the same way. `decode_unicode=False` is the point:
        // `text/event-stream` carries no charset, so `requests` would decode as ISO-8859-1 and
        // mangle every accented character — the clone decodes UTF-8 itself, per line, below.
        match pull_line(response, &mut buffer, idle).await {
            Ok(true) => {}
            Ok(false) => break, // the body ended; 1150 returns what survived
            Err(failure) => return Err(failure),
        }
        if deadline.is_some_and(|when| Instant::now() > when) {
            return Err(StreamFailure::Timeout(StreamTimeout));
        }
        let Some(line) = take_line(&mut buffer) else {
            continue;
        };
        if apply_line(&line, request_id, on_delta, &mut read) {
            break; // `data: [DONE]` (1111-1112): the rest of the socket is not read.
        }
    }
    Ok(read)
}

/// Pull bytes until the buffer holds a newline, or the body ends. `Ok(true)` means a line is
/// ready to take; `Ok(false)` means the body is over, with whatever remains in the buffer.
///
/// `idle` is the source's read timeout (`agent.py:1176`'s `timeout=(idle, idle)`), which restarts on
/// every chunk: each wait for the next piece of the body gets the whole window again, and a window
/// that expires is the `ReadTimeout` the source raises from inside `iter_lines` — retryable, because
/// it is a `Timeout`, and unlike the deadline's `RuntimeError` of 1098.
async fn pull_line(
    response: &mut Response,
    buffer: &mut Vec<u8>,
    idle: Option<Duration>,
) -> Result<bool, StreamFailure> {
    loop {
        if buffer.contains(&b'\n') {
            return Ok(true);
        }
        let chunk = match idle {
            Some(window) => timeout(window, response.chunk())
                .await
                .map_err(|_| read_timeout_failure(window))?
                .map_err(StreamFailure::transport)?,
            None => response.chunk().await.map_err(StreamFailure::transport)?,
        };
        match chunk {
            Some(chunk) => buffer.extend_from_slice(&chunk),
            None => return Ok(false),
        }
    }
}

/// The `ReadTimeout` of an idle window that expired, as the `io::Error` of kind `TimedOut` — which
/// `is_transient_stream_failure` reads as retryable, the way `requests`' `ReadTimeout` is. The
/// wording is the clone's own, because the source's is urllib3's pool description; `docs/PARITY.md`
/// records it. `Duration`'s own `Debug` names the window exactly (`150ms`, `2.5s`), which is what a
/// log reader wants to see next to the reply budget it expired.
fn read_timeout_failure(window: Duration) -> StreamFailure {
    StreamFailure::transport(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("the engine sent no bytes for {window:?}"),
    ))
}

/// Split the first `\n`-terminated line off the buffer, or the unterminated tail at end of body —
/// the pair `iter_lines` yields: complete lines, then a final partial one if bytes remain.
/// A `\r` is left in for `apply_line`'s `trim` to remove, which is what Python's `strip` does with
/// it on a CRLF body.
fn take_line(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    if let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
        let line = buffer[..index].to_vec();
        buffer.drain(..=index);
        return Some(line);
    }
    if buffer.is_empty() {
        return None;
    }
    Some(std::mem::take(buffer))
}

/// One line through the source's filter chain (1099-1149): blank, comment, non-`data:`, empty
/// payload, unparsable JSON, non-object — each skipped in the source's order — and then `usage`,
/// `choices[0].finish_reason`, and the `delta` (or `message`) content piece. Returns whether the
/// line was the `[DONE]` sentinel.
fn apply_line(
    raw: &[u8],
    request_id: &str,
    on_delta: DeltaSink<'_>,
    read: &mut StreamRead,
) -> bool {
    // `decode("utf-8", errors="replace").strip()` (1102): the replace keeps a byte the engine
    // split across two chunks from failing the line, and `str::trim` matches Python's whole
    // whitespace set.
    let line = String::from_utf8_lossy(raw);
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || !line.starts_with("data:") {
        return false;
    }
    let data = line["data:".len()..].trim();
    if data.is_empty() {
        return false;
    }
    if data == "[DONE]" {
        return true;
    }

    let Ok(event) = serde_json::from_str::<Value>(data) else {
        debug!(
            "[{request_id}] Skipping unparsable LM stream event: {}",
            safe_body_snippet(data)
        );
        return false;
    };
    if !event.is_object() {
        return false;
    }

    if let Some(usage) = event.get("usage").filter(|value| value.is_object()) {
        read.usage = usage.clone();
    }

    // `if not isinstance(choices, list) or not choices: continue` (1125-1127).
    let Some(first) = event
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return false;
    };
    // `choices[0] if isinstance(choices[0], dict) else {}` (1128): `Value::get` answers `None` on a
    // non-object, which is exactly the source's `{}` standing in for one, so the choice is handed on
    // as it was read.
    apply_choice(first, on_delta, read)
}

/// The `choices[0]` half of the chain (1128-1149): the finish reason, then the content piece.
/// Returns whether the choice carried a piece.
fn apply_choice(choice: &Value, on_delta: DeltaSink<'_>, read: &mut StreamRead) -> bool {
    // `if choice.get("finish_reason")` (1129): only a truthy one replaces, so a later `null` or
    // empty string keeps the reason an earlier event gave.
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        if !reason.is_empty() {
            read.finish_reason = reason.to_string();
        }
    }

    let mut delta = choice.get("delta").filter(|value| value.is_object());
    if delta.is_none() {
        // Servers that stream in the non-streaming shape put the text in `message` instead (1134).
        delta = choice.get("message").filter(|value| value.is_object());
    }
    let Some(piece) = delta
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    if piece.is_empty() {
        return false;
    }

    read.content.push_str(piece);
    read.pieces.push(piece.to_string());
    // The source wraps this call in try/except so a failing progress sink cannot cost the reply
    // (1143-1148); a `Fn` closure cannot fail, so there is nothing to catch — the divergence row
    // in `docs/PARITY.md` records it.
    on_delta(piece);
    false
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use shed_config::{EnvSource, LmEngine};

    use super::*;

    /// One scripted answer from the fake engine, covering the shapes `test_lm_streaming.py`
    /// scripts with its `FakeStreamResponse` and its `side_effect` lists: a streamed body, a
    /// body sent in named pieces, a status that rejects the stream, and an engine that accepts
    /// and says nothing.
    ///
    /// A real engine does not stop answering because one request went badly, so the last script of
    /// a list is its standing behaviour and every earlier one is a single scripted failure — see
    /// `next_script`.
    #[derive(Clone)]
    enum Script {
        /// 200 `text/event-stream`, the whole body in one write.
        Body(Vec<u8>),
        /// 200 `text/event-stream`, each piece flushed with a pause before it, so the reader
        /// really does see the boundaries between them. `promised` is what `Content-Length`
        /// claims: promise more than the pieces hold and the body ends early — the source's
        /// connection dropped mid-stream.
        Chunks {
            pieces: Vec<Vec<u8>>,
            promised: usize,
        },
        /// A status with a body and no stream — the rejection of 1183.
        Status {
            code: u16,
            body: Vec<u8>,
            content_type: &'static str,
        },
        /// Read the request, then hold the connection open saying nothing, until the client's
        /// own timeout gives up.
        Silent,
    }

    /// The fake engine: a real HTTP/1.1 listener, so the port's own transport — reqwest, the
    /// chunk reader, the ladder — is what the tests exercise, and a counter of the requests it
    /// answered, which is how the tests read `post.call_count` the way the Python suite does.
    struct FakeEngine {
        host: String,
        requests: Arc<AtomicUsize>,
    }

    impl FakeEngine {
        fn count(&self) -> u64 {
            u64::try_from(self.requests.load(Ordering::Relaxed)).unwrap_or(u64::MAX)
        }

        /// The same listener with the ladder's settings bolted on, which is what every call needs.
        fn transport(&self, retries: u64) -> LmTransport {
            transport_for(&self.host, retries)
        }
    }

    /// A transport pointed at `host` with the ladder's settings on it.
    ///
    /// The waits themselves are tested by `backoff_ms`'s own suite in `retry.rs`; the ladder here
    /// runs at the floor base (`agent.py:516`, `max(50, …)`) and no jitter, so a retried test is
    /// quick and never sleeps on a coin toss.
    fn transport_for(host: &str, retries: u64) -> LmTransport {
        let mut values = BTreeMap::new();
        values.insert("LM_ENGINE_HOST".to_string(), host.to_string());
        values.insert(
            "LM_ENGINE_TRANSIENT_RETRIES".to_string(),
            retries.to_string(),
        );
        values.insert("LM_ENGINE_RETRY_BASE_MS".to_string(), "50".to_string());
        values.insert("LM_ENGINE_RETRY_JITTER_MS".to_string(), "0".to_string());
        LmTransport::new(&LmEngine::from_source(&EnvSource::from_file_values(values)))
    }

    /// A port nothing answers: reserved and released, so a connect to it is refused rather than
    /// merely left unanswered — `requests.exceptions.ConnectionError` in the source, reqwest's
    /// connect error here.
    async fn dead_port() -> SocketAddr {
        let spacer = match TcpListener::bind("127.0.0.1:0").await {
            Ok(spacer) => spacer,
            Err(error) => panic!("could not reserve a port: {error}"),
        };
        match spacer.local_addr() {
            Ok(address) => address,
            Err(error) => panic!("the spacer has no address: {error}"),
        }
    }

    /// Start a fake engine answering one script per request, the last script repeated once the
    /// list runs out — `side_effect=[a, b]` semantics.
    async fn fake_engine(scripts: Vec<Script>) -> FakeEngine {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) => panic!("the fake engine could not bind: {error}"),
        };
        let address = match listener.local_addr() {
            Ok(address) => address,
            Err(error) => panic!("the fake engine has no address: {error}"),
        };
        let scripts = Arc::new(Mutex::new(VecDeque::from(scripts)));
        let requests = Arc::new(AtomicUsize::new(0));
        tokio::spawn(accept_loop(listener, scripts, Arc::clone(&requests)));
        FakeEngine {
            host: format!("http://127.0.0.1:{}", address.port()),
            requests,
        }
    }

    async fn accept_loop(
        listener: TcpListener,
        scripts: Arc<Mutex<VecDeque<Script>>>,
        requests: Arc<AtomicUsize>,
    ) {
        loop {
            let (socket, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return, // The runtime is gone; the test is over.
            };
            let scripts = Arc::clone(&scripts);
            let requests = Arc::clone(&requests);
            tokio::spawn(async move {
                let script = match scripts.lock() {
                    Ok(mut queue) => next_script(&mut queue),
                    Err(poisoned) => next_script(&mut poisoned.into_inner()),
                };
                requests.fetch_add(1, Ordering::Relaxed);
                if let Some(script) = script {
                    let _ = answer(socket, script).await; // A client given a script hangs up early.
                }
            });
        }
    }

    /// The script for the next request: the queue drains front-to-back, and the last script stays
    /// for good — `side_effect=[a, b]` answers `a`, then `b`, then `b` again, which is what a
    /// retried request against an engine with a standing behaviour looks like.
    fn next_script(queue: &mut VecDeque<Script>) -> Option<Script> {
        if queue.len() > 1 {
            return queue.pop_front();
        }
        queue.front().cloned()
    }

    /// Read the POST in full — the source's server would not answer a request still being
    /// written — then play the script.
    async fn answer(mut socket: TcpStream, script: Script) -> std::io::Result<()> {
        read_request(&mut socket).await?;
        match script {
            Script::Body(body) => {
                socket
                    .write_all(&response_head(200, "OK", "text/event-stream", body.len()))
                    .await?;
                socket.write_all(&body).await?;
            }
            Script::Chunks { pieces, promised } => {
                socket
                    .write_all(&response_head(200, "OK", "text/event-stream", promised))
                    .await?;
                for piece in pieces {
                    // The pause is what keeps the pieces apart on the wire; a writer that
                    // flushes back-to-back lets the kernel coalesce them, and the test would
                    // silently stop testing the boundary.
                    sleep(Duration::from_millis(40)).await;
                    socket.write_all(&piece).await?;
                    socket.flush().await?;
                }
            }
            Script::Status {
                code,
                body,
                content_type,
            } => {
                socket
                    .write_all(&response_head(
                        code,
                        reason_for(code),
                        content_type,
                        body.len(),
                    ))
                    .await?;
                socket.write_all(&body).await?;
            }
            Script::Silent => hold_open(&mut socket).await,
        }
        socket.flush().await
    }

    /// Drain the request the client is writing: the request line and headers to the blank line, then
    /// the body its `Content-Length` promises. A server that answered mid-request would have the rest
    /// of the request arrive after its own reply, which is not how the source's fake server works.
    async fn read_request(socket: &mut TcpStream) -> std::io::Result<()> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if socket.read(&mut byte).await? == 0 {
                return Ok(()); // The client hung up before asking anything.
            }
            head.push(byte[0]);
        }
        let lowered = String::from_utf8_lossy(&head).to_lowercase();
        let length = lowered
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        if length > 0 {
            socket.read_exact(&mut body).await?;
        }
        Ok(())
    }

    /// The reason phrase of a status the scripts use. Only the status line is parsed by a client, so
    /// an approximate phrase costs nothing; an unknown code answers as the 503 the ladder retries.
    fn reason_for(code: u16) -> &'static str {
        match code {
            400 => "Bad Request",
            408 => "Request Timeout",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "Service Unavailable",
        }
    }

    /// The engine that thinks forever: the request is read, and the reply never starts. The client's
    /// own window is what ends this, as it ends the source's — so the wait here is not a blind sleep
    /// but the client hanging up, which is the end of the conversation. Reading until EOF is what
    /// makes the test cost its idle window rather than a fixed guess at one.
    async fn hold_open(socket: &mut TcpStream) {
        let mut drained = [0u8; 512];
        loop {
            match socket.read(&mut drained).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }

    fn response_head(code: u16, reason: &str, content_type: &str, length: usize) -> Vec<u8> {
        format!("HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
            .into_bytes()
    }

    /// The sink the tests listen through, counting the pieces in order.
    #[derive(Default)]
    struct Heard(Arc<Mutex<Vec<String>>>);

    impl Heard {
        fn sink(&self) -> impl Fn(&str) {
            let heard = Arc::clone(&self.0);
            move |piece: &str| match heard.lock() {
                Ok(mut pieces) => pieces.push(piece.to_string()),
                Err(poisoned) => poisoned.into_inner().push(piece.to_string()),
            }
        }

        fn pieces(&self) -> Vec<String> {
            match self.0.lock() {
                Ok(pieces) => pieces.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        }
    }

    /// `sse(*events)` (`test_lm_streaming.py:11`): events as a server streams them.
    fn sse(events: &[Value]) -> Vec<u8> {
        let mut body = String::new();
        for event in events {
            body.push_str("data: ");
            if event == &json!("[DONE]") {
                body.push_str("[DONE]");
            } else {
                body.push_str(&event.to_string());
            }
            body.push_str("\n\n");
        }
        body.into_bytes()
    }

    /// `delta_event(text, finish_reason=None)` (`test_lm_streaming.py:23`).
    fn delta_event(text: &str) -> Value {
        json!({"choices": [{"delta": {"content": text}, "finish_reason": null}]})
    }

    /// A `reply`-shaped delta carrying `text` verbatim, the way every JSON-reply turn ends.
    fn reply(text: &str) -> Value {
        delta_event(&format!("{{\"reply\": {text}}}"))
    }

    async fn read_all(
        engine: &FakeEngine,
        retries: u64,
        idle: Option<Duration>,
        deadline: Option<Instant>,
        heard: &Heard,
    ) -> Result<StreamRead, LadderError> {
        let transport = engine.transport(retries);
        call_lm_http_stream_chat(
            &transport,
            "unit test",
            &json!({"prompt": "ping", "max_tokens": 8}),
            idle,
            deadline,
            &heard.sink(),
        )
        .await
    }

    #[tokio::test]
    async fn the_streamed_reply_is_assembled_from_deltas() {
        let body = sse(&[
            delta_event("{\"reply\": "),
            delta_event("\"Streamed.\"}"),
            json!({
                "choices": [{"delta": {}, "finish_reason": "stop"}],
                "usage": {"completion_tokens": 11, "prompt_tokens": 22}
            }),
            json!("[DONE]"),
            delta_event("never read"), // past [DONE], as the source stops at 1110.
        ]);
        let engine = fake_engine(vec![Script::Body(body)]).await;
        let heard = Heard::default();

        let read = match read_all(&engine, 0, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("the stream should have been read: {error}"),
        };

        // The same shape the non-streaming JSON path hands back (1084-1087).
        assert_eq!(read.content, "{\"reply\": \"Streamed.\"}");
        assert_eq!(read.finish_reason, "stop");
        assert_eq!(read.usage["completion_tokens"], json!(11));
        assert_eq!(read.usage["prompt_tokens"], json!(22));
        assert_eq!(
            read.pieces,
            vec!["{\"reply\": ".to_string(), "\"Streamed.\"}".to_string()]
        );
        assert_eq!(
            heard.pieces(),
            read.pieces,
            "the sink hears exactly the pieces"
        );
        assert_eq!(engine.count(), 1, "one request, the [DONE] was its end");
    }

    #[tokio::test]
    async fn garbage_events_are_skipped_without_losing_the_reply() {
        let mut body = sse(&[delta_event("{\"reply\": ")]);
        // The lines the source's filter chain drops: not JSON at all, then a non-object, a
        // `choices` that is not a list, and an object with nothing useful in it (1115-1127).
        body.extend_from_slice(b"data: {oops\n\n");
        body.extend_from_slice(b"data: {\"choices\": \"not a list\"}\n\n");
        body.extend_from_slice(b"data: {\"nothing\": \"useful\"}\n\n");
        body.extend_from_slice(&sse(&[delta_event("\"Kept.\"}"), json!("[DONE]")]));
        let engine = fake_engine(vec![Script::Body(body)]).await;
        let heard = Heard::default();

        let read = match read_all(&engine, 0, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("garbage must cost the reply nothing: {error}"),
        };

        assert_eq!(read.content, "{\"reply\": \"Kept.\"}");
        assert_eq!(read.pieces.len(), 2, "only the two deltas were spoken");
    }

    #[tokio::test]
    async fn comment_and_blank_lines_are_ignored() {
        // The keep-alive comment and the blank separators a real SSE body is made of (1103).
        let body = b": keep-alive\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"{\\\"reply\\\": \\\"Fine.\\\"}\"}}]}\n\ndata: [DONE]\n";
        let engine = fake_engine(vec![Script::Body(body.to_vec())]).await;
        let heard = Heard::default();

        let read = match read_all(&engine, 0, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("a keep-alive must not disturb the read: {error}"),
        };

        assert_eq!(read.content, "{\"reply\": \"Fine.\"}");
    }

    #[tokio::test]
    async fn a_message_shaped_chunk_is_read_like_a_delta() {
        // Some servers stream but still send the non-streaming `message` shape (1136).
        let body = sse(&[
            json!({"choices": [{"message": {"content": "{\"reply\": \"Shaped.\"}"}, "finish_reason": "stop"}]}),
            json!("[DONE]"),
        ]);
        let engine = fake_engine(vec![Script::Body(body)]).await;
        let heard = Heard::default();

        let read = match read_all(&engine, 0, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("the message shape should read like a delta: {error}"),
        };

        assert_eq!(read.content, "{\"reply\": \"Shaped.\"}");
        assert_eq!(read.finish_reason, "stop");
    }

    #[tokio::test]
    async fn accented_text_survives_a_split_across_chunks() {
        let body = sse(&[reply("\"Café.\""), json!("[DONE]")]);
        // Split inside the two-byte é, which is the byte pattern `iter_lines` must not lose.
        let split = match body.windows(2).position(|window| window == [0xC3, 0xA9]) {
            Some(index) => index + 1,
            None => panic!("the test body should hold an é"),
        };
        let engine = fake_engine(vec![Script::Chunks {
            pieces: vec![body[..split].to_vec(), body[split..].to_vec()],
            promised: body.len(),
        }])
        .await;
        let heard = Heard::default();

        let read = match read_all(&engine, 0, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("the split byte must not cost the reply: {error}"),
        };

        assert_eq!(read.content, "{\"reply\": \"Café.\"}");
    }

    #[tokio::test]
    async fn a_rejected_stream_reports_itself_as_unavailable() {
        // 1183: a status with nothing spoken is the caller's cue to fall back to the JSON path,
        // and — mirroring the non-streaming ladder — a status outside {502, 503, 504} is not
        // something to retry.
        let engine = fake_engine(vec![Script::Status {
            code: 500,
            body: b"boom".to_vec(),
            content_type: "text/html",
        }])
        .await;
        let heard = Heard::default();

        let error = match read_all(&engine, 2, None, None, &heard).await {
            Err(error) => error,
            Ok(read) => panic!("a 500 must not read as a reply: {:?}", read.content),
        };

        let unavailable = match error.downcast_ref::<LmStreamUnavailable>() {
            Some(unavailable) => unavailable,
            None => panic!("a rejection is the fallback signal, got: {error}"),
        };
        let text = unavailable.to_string();
        assert!(
            text.contains("status 500") && text.contains("boom"),
            "the rejection text of `_lm_stream_rejection` (1232): {text}"
        );
        assert_eq!(engine.count(), 1, "a rejection is not a transient failure");
        assert!(heard.pieces().is_empty(), "nothing was delivered");
    }

    #[tokio::test]
    async fn a_failure_after_a_delta_is_never_retried() {
        // 1201: once the turn has spoken, repeating it would speak it twice. The body promises
        // more than it holds and the connection ends — the source's mid-stream exception.
        let piece = sse(&[delta_event("{\"repl")]);
        let engine = fake_engine(vec![Script::Chunks {
            pieces: vec![piece],
            promised: 4096,
        }])
        .await;
        let heard = Heard::default();

        let error = match read_all(&engine, 2, None, None, &heard).await {
            Err(error) => error,
            Ok(read) => panic!(
                "a truncated body must not read as a reply: {:?}",
                read.content
            ),
        };

        assert!(
            error.downcast_ref::<LmStreamUnavailable>().is_none(),
            "a stream that already spoke is not a fallback signal: {error}"
        );
        assert_eq!(
            engine.count(),
            1,
            "the ladder must not ask a spoken turn to speak again"
        );
        assert_eq!(
            heard.pieces().len(),
            1,
            "the partial piece reached the sink"
        );
    }

    #[tokio::test]
    async fn a_failure_before_any_delta_is_retried() {
        // 1201's other side, the `test_a_failure_before_any_delta_is_retried` case: a 503 the
        // engine answers before saying anything is retried, and the second try is the reply.
        let engine = fake_engine(vec![
            Script::Status {
                code: 503,
                body: b"upstream down".to_vec(),
                content_type: "text/plain",
            },
            Script::Body(sse(&[reply("\"Back.\""), json!("[DONE]")])),
        ])
        .await;
        let heard = Heard::default();

        let read = match read_all(&engine, 1, None, None, &heard).await {
            Ok(read) => read,
            Err(error) => panic!("the retry should have reached the reply: {error}"),
        };

        assert_eq!(read.content, "{\"reply\": \"Back.\"}");
        assert_eq!(
            engine.count(),
            2,
            "one rejection, one retry — then the reply"
        );
    }

    #[tokio::test]
    async fn a_deadline_that_has_passed_stops_the_stream() {
        // 1097-1098: the deadline is checked before the first line, so a budget already spent
        // never reads a reply — and it is the raw RuntimeError, which the ladder never retries.
        let engine = fake_engine(vec![Script::Body(sse(&[
            reply("\"Late.\""),
            json!("[DONE]"),
        ]))])
        .await;
        let heard = Heard::default();
        let deadline = match Instant::now().checked_sub(Duration::from_secs(1)) {
            Some(deadline) => deadline,
            None => Instant::now(),
        };

        let error = match read_all(&engine, 2, None, Some(deadline), &heard).await {
            Err(error) => error,
            Ok(read) => panic!("a spent budget must not read a reply: {:?}", read.content),
        };

        // The raw raise of 1202: the deadline's `RuntimeError`, unwrapped, because the ladder's
        // 1201 re-raises whatever the reader surfaced.
        assert!(
            matches!(
                error.downcast_ref::<StreamFailure>(),
                Some(StreamFailure::Timeout(_))
            ),
            "the source's own words: {error}"
        );
        assert_eq!(
            error.to_string(),
            "Model generation timed out while streaming."
        );
        assert_eq!(engine.count(), 1, "a timeout is never retried");
        assert!(heard.pieces().is_empty());
    }

    #[tokio::test]
    async fn an_idle_engine_times_out_the_reader() {
        // The `ReadTimeout` of `timeout=(idle, idle)` (1176), on the wait for the head: an engine
        // that accepts and never answers is a `Timeout` to 1179, so the ladder spends its retry on
        // it, and then raises the 1182 wrapper — the request never produced a head to describe.
        let engine = fake_engine(vec![Script::Silent]).await;
        let heard = Heard::default();

        let error = match read_all(&engine, 1, Some(Duration::from_millis(150)), None, &heard).await
        {
            Err(error) => error,
            Ok(read) => panic!(
                "a silent engine must not read as a reply: {:?}",
                read.content
            ),
        };

        assert!(
            error.downcast_ref::<LmRequestError>().is_some(),
            "a POST that never got a head is the source's wrapper: {error}"
        );
        let text = error.to_string();
        assert!(
            text.starts_with("LM request failed during unit test: ") && text.contains("no bytes"),
            "the wait that expired should be named: {text}"
        );
        assert_eq!(
            engine.count(),
            2,
            "an expired window is a Timeout, which the ladder retries"
        );
        assert!(heard.pieces().is_empty());
    }

    #[tokio::test]
    async fn a_refused_connection_raises_the_ladders_wrapper() {
        // No listener at all: `requests.exceptions.ConnectionError` in the source, reqwest's
        // connect error here. One attempt only, and not because the retry is uninteresting —
        // `an_idle_engine_times_out_the_reader` already counts the two POSTs a transient head
        // failure costs. It is that this machine answers a refused loopback connect two seconds
        // after being asked, so a second attempt would buy the same fact for twice the wait. What
        // the retry *would* have done — spend itself on this error — is asserted below, on the
        // classifier the ladder consulted before deciding.
        let address = dead_port().await;
        let transport = transport_for(&format!("http://{address}"), 0);
        let heard = Heard::default();

        let error = match call_lm_http_stream_chat(
            &transport,
            "unit test",
            &json!({"prompt": "ping", "max_tokens": 8}),
            None,
            None,
            &heard.sink(),
        )
        .await
        {
            Err(error) => error,
            Ok(read) => panic!("nothing is listening: {:?}", read.content),
        };

        // 1182: a POST that never produced a response head is always the wrapper, whatever the
        // attempt count — nothing was ever delivered, so there is nothing the wrapper would
        // duplicate.
        assert!(
            error.downcast_ref::<LmRequestError>().is_some(),
            "the POST failure should be the source's wrapper: {error}"
        );
        assert!(
            error
                .to_string()
                .starts_with("LM request failed during unit test: "),
            "the wrapper names its purpose: {error}"
        );
        assert!(heard.pieces().is_empty(), "nothing was spoken");

        // The connect error is the wrapped cause, two links down: the wrapper's source is the
        // `StreamFailure` the POST surfaced, and its source is the transport's own error.
        let mut saw_connect = false;
        let mut saw_retryable_cause = false;
        let mut cause = error.source();
        while let Some(step) = cause {
            if let Some(transport_error) = step.downcast_ref::<reqwest::Error>() {
                saw_connect = transport_error.is_connect();
                saw_retryable_cause = is_transient_exception(transport_error);
            }
            cause = step.source();
        }
        assert!(
            saw_connect,
            "the cause is the transport's connect error: {error:?}"
        );
        assert!(
            saw_retryable_cause,
            "which is the `ConnectionError` of 1032, so the ladder would have retried it"
        );
    }

    #[test]
    fn the_stream_backoff_is_the_ladders_backoff() {
        // 1224 repeats 1051 verbatim; the floor base and no jitter make the doubling exact.
        let mut values = BTreeMap::new();
        values.insert("LM_ENGINE_RETRY_BASE_MS".to_string(), "50".to_string());
        values.insert("LM_ENGINE_RETRY_JITTER_MS".to_string(), "0".to_string());
        let engine = LmEngine::from_source(&EnvSource::from_file_values(values));

        assert_eq!(backoff_ms(&engine, 1), 50);
        assert_eq!(backoff_ms(&engine, 2), 100);
        assert_eq!(backoff_ms(&engine, 3), 200);
    }

    #[test]
    fn an_idle_window_reads_as_transient() {
        // The wait the reader raises on an expired window must join `requests`' `ReadTimeout`
        // in the retryable set (1201 reads it as a `Timeout`), while an unrelated io error —
        // the reset of a connection the engine dropped — must not.
        let timeout = io::Error::new(io::ErrorKind::TimedOut, "the engine sent no bytes for 1s");
        assert!(
            is_transient_stream_failure(&StreamFailure::transport(timeout)),
            "an expired idle window is the source's ReadTimeout, which is a Timeout"
        );

        let reset = io::Error::new(io::ErrorKind::ConnectionReset, "connection reset");
        assert!(
            !is_transient_stream_failure(&StreamFailure::transport(reset)),
            "a reset body is not the source's Timeout"
        );
    }

    #[test]
    fn the_unavailable_error_is_the_sources_words() {
        // `str(RuntimeError(msg))` is the message alone, and the message is what the caller at
        // 7016 puts in its log and its fallback reason — so the type must not decorate it.
        let detail = "status 503 with content_type text/plain: boom";
        let error = LmStreamUnavailable {
            detail: detail.to_string(),
        };
        assert_eq!(error.to_string(), detail);
    }
}
