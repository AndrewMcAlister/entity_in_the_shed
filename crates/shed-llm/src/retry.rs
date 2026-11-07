//! The retry ladder in front of every LM Studio HTTP call.
//!
//! Ported from `agent.py:1038` (`call_lm_http_with_retry`) and `1030` (`_is_retryable_lm_failure`).
//! Stage 2 has one caller for it, `discover_lm_studio_models` (`1549`); the chat paths — the
//! non-streaming `call_lm_engine` and the streaming ladder in [`crate::stream`] — arrive at Stage 4.
//!
//! ## The ladder, exactly as the source builds it
//!
//! `retries = LM_ENGINE_TRANSIENT_RETRIES` (1039, default 2) counts *re*tries, and
//! `while attempt <= retries` (1044) with `attempt += 1` at the top (1045) means the request is
//! made `retries + 1` times. The wait before attempt `n` is
//! `LM_ENGINE_RETRY_BASE_MS * 2^(n-1) + randint(0, LM_ENGINE_RETRY_JITTER_MS)` (1051, 1060) —
//! 250-400 ms, then 500-650 ms with the shipped defaults.
//!
//! ## The tail at 1070 is unreachable
//!
//! `raise RuntimeError(f"LM request failed during {purpose}: {last_error}")` sits after the loop,
//! but every exit from the loop is a `return` (1068), a `raise` (1057) or a `continue` (1056, 1066),
//! and the `continue` on the last allowed attempt fails its own guard (`attempt <= retries` at
//! 1050 and 1059, with `attempt` already incremented) and raises at 1057 instead. So the tail can
//! only be reached by a loop that never ran, which needs `retries < 0`, which `max(0, …)` at 491
//! rules out. `last_error` (1042) is written and never read.
//!
//! The Rust port therefore has no counterpart to either: the exhaustion path is the `raise` at
//! 1057, which is where a real failure is reported, and the tail is `unreachable!` with this
//! paragraph beside it. Nothing observable changes, so the choice is recorded in
//! `docs/PARITY.md`, "Structural changes" rather than as a divergence.
//!
//! ## Which failures count as transient
//!
//! `_is_retryable_lm_failure` (1030) retries two things: an exception that is
//! `requests.exceptions.Timeout` or `requests.exceptions.ConnectionError`, and a response whose
//! status is 502, 503 or 504 (1032-1035). Anything else — a 400, a 500, a malformed URL — is
//! **returned** rather than raised, which is why the discovery pass checks the status itself
//! (`1554`) instead of catching something.

use std::time::Duration;

use rand::Rng;
use reqwest::{Client, Error as ReqwestError, Method, Response};
use tokio::time::sleep;
use tracing::warn;

use shed_config::LmEngine;

/// `agent.py:1035`'s set. The engine answered, but it could not answer *now*: 502 is an upstream
/// that refused, 503 is an engine still loading, 504 is one that took too long to answer. A 500 is
/// not in the set, because an engine that throws on a request will throw on the identical request
/// again, and a 400 means the request is wrong rather than late.
const TRANSIENT_STATUSES: [u16; 3] = [502, 503, 504];

/// A request that never produced a response: `agent.py:1057`'s `RuntimeError`, which is what a
/// caller sees when the ladder runs out or when the failure was never transient to begin with.
///
/// The wording is verbatim, including the `{exc}` it quotes, because a caller that shows this to a
/// user shows this text: `api_error_response(str(exc), …)` on the routes that wrap it.
#[derive(Debug)]
pub struct LmRequestError {
    purpose: String,
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl LmRequestError {
    /// The `RuntimeError(f"LM request failed during {purpose}: {exc}")` the source raises when a
    /// ladder runs out (`agent.py:1057`, and the streaming ladder's two raisings at 1182 and 1210).
    /// The source is boxed rather than a `ReqwestError` because the streaming ladder wraps failures
    /// that are not reqwest errors too — a streamed response head that never arrives is a tokio
    /// elapsed, where the source's `requests` raises the `ReadTimeout` whose text the message quotes.
    /// The `Into` is for the same reason: the streaming ladder's last failure is sometimes only the
    /// text it printed (`stream.rs` keeps the string the warning line named, which is what 1210
    /// quotes), and a `String` is already boxable as an error.
    pub(crate) fn new(
        purpose: &str,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        Self {
            purpose: purpose.to_string(),
            source: source.into(),
        }
    }
}

// The house error style (`shed-json`'s `JsonError`, `shed-config`'s `ConfigError`): a hand-written
// `Display` that spells the source's f-string, and an `Error` impl whose `source` keeps the failure
// reachable for a caller that wants to know *which* transient failure ran the ladder out.
impl std::fmt::Display for LmRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LM request failed during {}: {}",
            self.purpose, self.source
        )
    }
}

impl std::error::Error for LmRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // Deref the box rather than `as_ref` it: `Box<dyn Error>` only conditionally implements
        // `Error`, and that condition wants a sized inner type, which a trait object is not.
        Some(&*self.source)
    }
}

/// The transport every LM Studio request goes through: one client, the ladder's three numbers, and
/// the address the source builds its URLs from.
///
/// `requests.request` (1047) has no session, so nothing is shared between calls in the source
/// either; sharing a client is the clone's own choice, and it is the one reqwest asks for, because
/// a client owns a connection pool and a TLS config that would otherwise be rebuilt per request.
/// It changes nothing the ladder decides.
#[derive(Debug)]
pub struct LmTransport {
    engine: LmEngine,
    client: Client,
}

impl LmTransport {
    /// Build the transport from the engine settings.
    ///
    /// The client is built with no default timeout of its own, because in the source the timeout is
    /// an argument of the call (`timeout=AI_MODEL_DISCOVERY_TIMEOUT` at 1549) rather than a property
    /// of the session, and a default here would silently apply to a call that forgot to name one.
    ///
    /// `build()` cannot fail for this client — there is no TLS backend to initialise, because every
    /// URL it is given is `http://` from `LM_ENGINE_HOST` — so the error branch is a fallback rather
    /// than a `Result` the boot would have to handle.
    pub fn new(engine: &LmEngine) -> Self {
        Self {
            engine: engine.clone(),
            client: Client::builder().build().unwrap_or_else(|_| Client::new()),
        }
    }

    /// `LM_ENGINE_HOST` as the source holds it: already stripped of its trailing slashes at
    /// `agent.py:393`, which is why the URL below appends `/v1/models` rather than joining paths.
    pub(crate) fn host(&self) -> &str {
        &self.engine.host
    }

    /// The shared client, for the one call shape this transport does not own: the streamed chat
    /// completion, whose per-attempt request `stream.rs` builds itself because it carries a
    /// `stream: true` body rather than being a bare `GET`.
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The engine settings, for the streaming ladder's own copy of the same three numbers
    /// (`stream.rs`): the same ladder, the same waits, read from the same place.
    pub(crate) fn engine(&self) -> &LmEngine {
        &self.engine
    }

    /// `AI_MODEL_DISCOVERY_TIMEOUT` (`agent.py:455`), in seconds, with `max(1, …)` already applied
    /// where the setting was read. It is the timeout the discovery call names, and it is a socket
    /// timeout in the source rather than a budget for the whole exchange — `docs/PARITY.md`,
    /// "Structural changes".
    pub(crate) fn discovery_timeout_seconds(&self) -> u64 {
        self.engine.discovery_timeout_seconds
    }

    /// `call_lm_http_with_retry`: one request, up to `retries + 1` attempts, and a response that is
    /// returned whether or not its status is a success.
    ///
    /// `purpose` is the phrase the warning line and the failure message name the call by —
    /// `"model discovery"` at `agent.py:1549`, `"chat completion"` at the Stage-4 call sites. It is
    /// not decoration: it is the only thing that tells a reader *which* LM request failed.
    ///
    /// `timeout` is the per-call budget, and it is the clone's reading of `requests`'
    /// `timeout=AI_MODEL_DISCOVERY_TIMEOUT` (`agent.py:1549`). The source's number is a socket
    /// timeout that restarts on every read, so a slow-but-alive engine survives it indefinitely;
    /// reqwest's `.timeout` is a total budget for the whole exchange. The divergence is a row in
    /// `docs/PARITY.md`, "Structural changes", and it only bites a call whose body takes longer
    /// than the number to arrive, which for this ladder is only ever the model list.
    pub async fn call(
        &self,
        request_id: &str,
        method: Method,
        url: &str,
        timeout: Duration,
        purpose: &str,
    ) -> Result<Response, LmRequestError> {
        let retries = self.engine.transient_retries;
        // 1053 and 1063 print `retries + 1`, which is the number of attempts, not of retries.
        let attempts = retries + 1;
        // `u64` rather than `u32` because the source has no ceiling on `LM_ENGINE_TRANSIENT_RETRIES`
        // (491 clamps it at zero and nothing else bounds it), and a counter that overflowed on the
        // way to an unreachable number of attempts would be the clone inventing a failure the source
        // does not have.
        let mut attempt: u64 = 0;

        while attempt <= retries {
            attempt += 1;

            let response = match self
                .client
                .request(method.clone(), url)
                .timeout(timeout)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    // 1050: the guard is on the exception *and* on the attempts left, and the
                    // attempts half is what makes the last attempt raise rather than sleep.
                    if is_transient_exception(&error) && attempt <= retries {
                        let backoff = self.wait_before_retry(
                            request_id,
                            purpose,
                            attempt,
                            attempts,
                            &Transient::Exception(error.to_string()),
                        );
                        sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                    return Err(LmRequestError::new(purpose, error));
                }
            };

            // 1059: a transient status is retried, and a non-transient one is handed back to the
            // caller with its status intact.
            let status = response.status().as_u16();
            if is_transient_status(status) && attempt <= retries {
                let backoff = self.wait_before_retry(
                    request_id,
                    purpose,
                    attempt,
                    attempts,
                    &Transient::Status(status),
                );
                sleep(Duration::from_millis(backoff)).await;
                continue;
            }

            return Ok(response);
        }

        unreachable!(
            "the ladder always leaves the loop through a return or a raise: `retries` is `max(0, …)` \
             (agent.py:491), so the loop runs at least once, and the attempt that runs out of retries \
             fails its own guard at 1050 or 1059 and raises"
        )
    }

    /// Write the warning line and return the wait it names, which is the pair 1052-1055 and
    /// 1061-1065 are: the line says how long the app is about to wait, so the number in the line
    /// and the number handed to `sleep` are the same number.
    ///
    /// The two lines the source writes are not the same sentence — an exception ends with `: {exc}`
    /// and a status says `status={code},` before the wait — so `Transient` carries which one this
    /// is, and the sentence is assembled from the part they share plus the part each one adds.
    fn wait_before_retry(
        &self,
        request_id: &str,
        purpose: &str,
        attempt: u64,
        attempts: u64,
        transient: &Transient,
    ) -> u64 {
        let backoff = backoff_ms(&self.engine, attempt);
        let line = match transient {
            Transient::Exception(reason) => format!(
                "[{request_id}] LM transient exception during {purpose} attempt={attempt}/{attempts}; \
                 retrying in {backoff}ms: {reason}"
            ),
            Transient::Status(status) => format!(
                "[{request_id}] LM transient status during {purpose} attempt={attempt}/{attempts}; \
                 status={status}, retrying in {backoff}ms"
            ),
        };
        warn!("{line}");
        backoff
    }
}

/// Which of the two transient failures the source distinguishes at 1048 and 1059: an exception
/// raised by the request, or a response head that names a status it should not have.
enum Transient {
    Exception(String),
    Status(u16),
}

/// `_is_retryable_lm_failure`'s exception half (`agent.py:1031-1032`): `Timeout` or
/// `ConnectionError`.
///
/// `requests` raises `ConnectTimeout` and `ReadTimeout` as `Timeout`, and `ConnectionError` for a
/// refused connection, a dropped one and a `RemoteDisconnected`. reqwest reports all of those as
/// `is_connect()` / `is_timeout()` / `is_request()`, and `is_request()` is the one that covers a
/// connection closed before the response head arrived, which `requests` calls `ConnectionError` and
/// which is transient for exactly the same reason: the engine never had a chance to say no.
///
/// A builder or URL error is `is_builder()`, which is not in the set — `requests` raises
/// `MissingSchema` for a malformed URL, and that is neither of the two classes at 1032 either.
///
/// The streamed ladder widens this set rather than editing it: `stream.rs` explains why a body
/// error is transient *there* but not here.
pub(crate) fn is_transient_exception(error: &ReqwestError) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request()
}

/// `_is_retryable_lm_failure`'s response half (`agent.py:1033-1035`).
pub(crate) fn is_transient_status(status: u16) -> bool {
    TRANSIENT_STATUSES.contains(&status)
}

/// `LM_ENGINE_RETRY_BASE_MS * 2 ** (attempt - 1) + random.randint(0, LM_ENGINE_RETRY_JITTER_MS)`
/// (1051, 1060), and the same expression again at 1224 for the streaming ladder.
///
/// `attempt` is 1-based, so the first wait is the base itself. `randint(a, b)` is inclusive at both
/// ends, which is `0..=` here: a jitter of 0 is a wait of exactly the base, and a jitter of 150 can
/// add 150. The source's arithmetic is exact integer arithmetic and so is this.
pub(crate) fn backoff_ms(engine: &LmEngine, attempt: u64) -> u64 {
    // `attempt` is 1-based, so the first wait is the base itself. The shift is capped at 63 rather
    // than left to grow: `LM_ENGINE_TRANSIENT_RETRIES` has no ceiling in the source (491 clamps it
    // at zero and nothing else), and Python's `2 ** 64` is simply a big integer, so the honest
    // translation of "the wait is now larger than a machine counts" is the largest wait there is.
    let shift = attempt.saturating_sub(1).min(63);
    let exponential = engine.retry_base_ms.saturating_mul(1_u64 << shift);
    // `randint(a, b)` is inclusive at both ends, so a jitter of 0 is a wait of exactly the base and
    // a jitter of 150 can add all 150.
    let jitter = rand::rng().random_range(0..=engine.retry_jitter_ms);
    exponential.saturating_add(jitter)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shed_config::EnvSource;

    use super::*;

    /// Settings written into the file-shaped source rather than the process environment, which is
    /// how every crate in this workspace hands a setting to a reader (`shed-config`'s own tests
    /// build the same `BTreeMap`).
    fn source(pairs: &[(&str, &str)]) -> EnvSource {
        EnvSource::from_file_values(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    /// The shipped ladder's settings, with the jitter switched off so a test can name the wait.
    /// Named for what it is rather than `engine`, which the tests' own bindings would shadow.
    fn engine_settings(retries: u64, base_ms: u64) -> LmEngine {
        let mut engine = LmEngine::from_source(&source(&[]));
        engine.transient_retries = retries;
        engine.retry_base_ms = base_ms;
        engine.retry_jitter_ms = 0;
        engine
    }

    #[test]
    fn the_wait_doubles_from_the_base_each_attempt_because_that_is_what_the_source_multiplies_by() {
        let engine = engine_settings(4, 250);
        assert_eq!(
            backoff_ms(&engine, 1),
            250,
            "2 ** 0: the first wait is the base"
        );
        assert_eq!(backoff_ms(&engine, 2), 500);
        assert_eq!(backoff_ms(&engine, 3), 1000);
        assert_eq!(backoff_ms(&engine, 4), 2000);
    }

    #[test]
    fn the_floor_the_settings_put_on_the_base_is_kept_in_the_ladder() {
        // `max(50, …)` at agent.py:516 is applied when the setting is read; the ladder multiplies
        // what survived, so the smallest first wait this app can ever sleep is 50 ms.
        let engine = LmEngine::from_source(&source(&[
            ("LM_ENGINE_RETRY_BASE_MS", "1"),
            ("LM_ENGINE_RETRY_JITTER_MS", "0"),
        ]));
        assert_eq!(engine.retry_base_ms, 50);
        assert_eq!(backoff_ms(&engine, 1), 50);
        assert_eq!(backoff_ms(&engine, 2), 100);
    }

    #[test]
    fn a_jitter_of_zero_adds_nothing_and_a_jitter_of_n_can_add_all_of_n() {
        let engine = engine_settings(2, 100);
        assert_eq!(backoff_ms(&engine, 1), 100);

        let mut jittered = engine_settings(2, 100);
        jittered.retry_jitter_ms = 40;
        for _ in 0..64 {
            let wait = backoff_ms(&jittered, 1);
            assert!(
                (100..=140).contains(&wait),
                "randint(0, 40) is inclusive at both ends: {wait}"
            );
        }
    }

    #[test]
    fn a_ladder_that_never_ends_still_costs_the_floor_rather_than_wrapping_into_a_huge_wait() {
        // `LM_ENGINE_TRANSIENT_RETRIES` has no ceiling in the source, so `2 ** (attempt - 1)` can
        // overflow. Python would happily produce a big integer; the clone saturates, because a
        // wait that wrapped into zero would be a retry storm instead of a slow retry.
        let engine = engine_settings(64, u64::MAX / 2);
        assert_eq!(backoff_ms(&engine, 64), u64::MAX);
    }

    #[test]
    fn only_the_three_statuses_the_source_lists_are_transient() {
        for status in [502_u16, 503, 504] {
            assert!(
                is_transient_status(status),
                "{status} is in the set at agent.py:1035"
            );
        }
        // A 500 is an engine that threw, and a 400 is a request that is wrong; repeating either
        // asks for the same answer again. 500 is the one that looks transient and is not.
        for status in [200_u16, 400, 404, 408, 429, 500, 501, 505] {
            assert!(
                !is_transient_status(status),
                "{status} is not in the set at agent.py:1035"
            );
        }
    }
}
