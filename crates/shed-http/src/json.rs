//! The JSON the browser reads, in the exact bytes Flask's `jsonify` produced.
//!
//! Three rules cover every response in this module, and all three come from
//! the client's tolerance rather than our taste:
//!
//! * the body is compact, keys sorted, with a trailing newline. A
//!   `serde_json::Map` is a `BTreeMap` (the `preserve_order` feature is off
//!   workspace-wide), so the sorting is the serialiser's, not a step we add;
//! * non-ASCII leaves as `\uXXXX`, because `json.dumps` escapes it and the
//!   source never turns that off — see [`EnsureAscii`];
//! * `Content-Type` is `application/json` with no `; charset=utf-8`. Flask
//!   sends it that way; a client that compares the header verbatim would
//!   otherwise see a different response for the same data.
//!
//! The error shape is `api_error_response` (`agent.py:987-999`) — the one
//! envelope every `/api/` failure arrives in, so the page can read
//! `payload.error` without a `try` around `response.json()`.

use std::io;

use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue, Response, StatusCode};
use serde::Serialize as _;
use serde_json::ser::Formatter;
use serde_json::{Map, Serializer, Value};

/// `X-Request-ID`, the header `assign_request_id`/`attach_request_id`
/// (`agent.py:1015-1027`) put on every `/api/` response.
pub(super) fn request_id_header() -> HeaderName {
    HeaderName::from_static("x-request-id")
}

/// The body bytes `jsonify` writes: compact JSON plus the newline it appends.
pub(super) fn json_bytes(payload: &Value) -> Vec<u8> {
    // Serialised through [`EnsureAscii`] rather than `Value`'s `Display`, which
    // is compact but leaves non-ASCII alone. A failure here would mean the
    // writer — a `Vec` — refused the write, which cannot happen; the fallback
    // keeps the response shape intact if it ever did.
    let mut bytes = Vec::new();
    {
        let mut serializer = Serializer::with_formatter(&mut bytes, EnsureAscii);
        if payload.serialize(&mut serializer).is_err() {
            bytes.clear();
            bytes.extend_from_slice(b"null");
        }
    }
    bytes.push(b'\n');
    bytes
}

/// Flask's `ensure_ascii=True`, the default `json.dumps` flag the source never
/// overrides (`agent.py` builds every payload with `jsonify`).
///
/// Every character above U+007F leaves the Python app as a `\uXXXX` escape, so
/// a Norwegian voice is `Norwegian (Bokm\u00e5l Norway)` on the wire and a
/// Japanese reply is entirely backslash-u. This writes the same escapes,
/// including the surrogate pairs Python uses for characters beyond the BMP
/// (an emoji becomes `\ud83d\ude00`, six+six bytes, not four raw ones).
///
/// It is not a cosmetic difference. 75 of the 110 `characters/*.json` files
/// carry non-ASCII — em dashes, accents, the character's own name in its own
/// script — so `/api/characters` answered a different 200 OK for most
/// countries until this was ported, and the entity's Japanese replies would
/// have answered differently for every country from Stage 4 on. The JSON the
/// browser parses is the same either way; the bytes are the contract, and the
/// Stage-9 sweep compares bytes.
///
/// Only string contents go through here: `serde_json` still owns the quotes,
/// the backslashes and the control-character escapes (`\n`, `\t`, `\u0000` and
/// friends), which are the same in both encoders.
struct EnsureAscii;

impl Formatter for EnsureAscii {
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut needs_work = false;
        for byte in fragment.bytes() {
            if byte > 0x7F {
                needs_work = true;
                break;
            }
        }
        if !needs_work {
            // The common case: every key and most values are ASCII, and the
            // fragment goes out exactly as `serde_json` sliced it.
            return writer.write_all(fragment.as_bytes());
        }
        let mut escaped = Vec::with_capacity(fragment.len() + 16);
        for ch in fragment.chars() {
            let code = ch as u32;
            if code <= 0x7F {
                escaped.push(code as u8);
            } else if code <= 0xFFFF {
                write_escape_u(&mut escaped, code);
            } else {
                // Python's pair: UTF-16 high surrogate, then low.
                let scaled = code - 0x1_0000;
                write_escape_u(&mut escaped, 0xD800 + (scaled >> 10));
                write_escape_u(&mut escaped, 0xDC00 + (scaled & 0x3FF));
            }
        }
        writer.write_all(&escaped)
    }
}

/// One `\uXXXX`, lowercase hex, exactly as Python's encoder writes it.
fn write_escape_u(bytes: &mut Vec<u8>, code: u32) {
    bytes.push(b'\\');
    bytes.push(b'u');
    for shift in [12_u32, 8, 4, 0] {
        let digit = u8::try_from((code >> shift) & 0xF).unwrap_or(0);
        bytes.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        });
    }
}

/// `jsonify(payload)` at 200 OK.
pub(super) fn jsonify(payload: &Value) -> Response<Body> {
    json_response(payload, StatusCode::OK, None, None)
}

/// A JSON response with one extra header — the shape `api_error_response`
/// needs for `X-Request-ID`, and the reason this is a function rather than a
/// call to `IntoResponse for Json`.
pub(super) fn json_response(
    payload: &Value,
    status: StatusCode,
    header_name: Option<HeaderName>,
    header_value: Option<HeaderValue>,
) -> Response<Body> {
    let mut response = Response::new(Body::from(json_bytes(payload)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let (Some(name), Some(value)) = (header_name, header_value) {
        headers.insert(name, value);
    }
    response
}

/// `api_error_response` (`agent.py:987-999`).
///
/// The two `or` defaults are the source's: an empty message becomes
/// `"internal server error"` and an empty code `"internal_error"`, because a
/// handler that caught `str(exc)` off an exception with no text still has to
/// answer something a client can print.
///
/// `extra` is the source's `payload.update(extra)`, so an extra key colliding
/// with one of the four wins, exactly as it does in Python.
pub(super) fn api_error_response(
    message: &str,
    status: u16,
    code: &str,
    extra: Option<&Value>,
    request_id: &str,
) -> Response<Body> {
    let mut payload = Map::new();
    payload.insert("ok".to_owned(), Value::Bool(false));
    payload.insert(
        "error".to_owned(),
        Value::String(text_or(message, "internal server error")),
    );
    payload.insert(
        "code".to_owned(),
        Value::String(text_or(code, "internal_error")),
    );
    payload.insert(
        "request_id".to_owned(),
        Value::String(request_id.to_owned()),
    );
    if let Some(Value::Object(extra)) = extra {
        for (key, value) in extra {
            payload.insert(key.clone(), value.clone());
        }
    }

    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json_response(
        &Value::Object(payload),
        status,
        Some(request_id_header()),
        Some(header_value(request_id)),
    )
}

/// The `except` arm of `api_json_boundary` (`agent.py:1007-1010`) for the one
/// failure a Rust handler can genuinely inherit from Flask: a request body
/// that never reached the handler as data. `request.get_json()` raising is an
/// exception like any other, so the boundary turns it into
/// `internal server error`/`internal_error` at 500 and logs the real cause
/// with the request id in front of it — the client never sees the cause,
/// which is the point of the decorator.
pub(super) fn internal_error(path: &str, cause: &str, request_id: &str) -> Response<Body> {
    tracing::error!("[{request_id}] Unhandled API exception on {path}: {cause}");
    api_error_response("", 500, "internal_error", None, request_id)
}

/// `str(value or fallback)` for a `&str`.
fn text_or(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

/// A header value from text that may, in principle, contain bytes a header
/// cannot hold. A generated id is hex and an incoming one was validated on
/// the way in, so the fallback is unreachable in practice; it is here so that
/// an id can never be the reason a response is not sent.
fn header_value(text: &str) -> HeaderValue {
    HeaderValue::from_str(text).unwrap_or_else(|_| HeaderValue::from_static("unavailable"))
}

#[cfg(test)]
mod tests {
    use super::json_bytes;
    use serde_json::{json, Map, Value};

    fn body(payload: Value) -> String {
        let bytes = json_bytes(&payload);
        match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => panic!("the encoder must write utf-8: {error}"),
        }
    }

    #[test]
    fn non_ascii_leaves_as_an_escape() {
        // The measured Python answer: `json.dumps` writes lowercase `\uXXXX`
        // for every character above U+007F. This is the exact byte run the
        // running app ships for a Norwegian voice's friendly name.
        assert_eq!(
            body(json!({"name": "Bokmål"})),
            "{\"name\":\"Bokm\\u00e5l\"}\n"
        );
    }

    #[test]
    fn astral_characters_become_a_surrogate_pair() {
        // Python encodes beyond-BMP characters as two escapes, the UTF-16
        // pair; four raw bytes would be a body no Python response matches.
        assert_eq!(
            body(json!("😀")),
            "\"\\ud83d\\ude00\"\n",
            "U+1F600 is D83D DC00"
        );
    }

    #[test]
    fn the_short_escapes_stay_short() {
        // The characters serde_json and Python agree on are left to
        // serde_json: `\n`, `\t`, `\r`, `\"`, `\\` and `\u00XX` for the
        // remaining controls, which is json.dumps' table too.
        assert_eq!(
            body(json!("a\nb\t\"c\\d\u{7}")),
            "\"a\\nb\\t\\\"c\\\\d\\u0007\"\n"
        );
    }

    #[test]
    fn ascii_bytes_pass_through_untouched() {
        // The fast path must not corrupt the common case: keys, `/`, `+` and
        // every printable stay exactly as the source wrote them.
        let mut map = Map::new();
        map.insert("z".to_owned(), json!(1));
        map.insert("a".to_owned(), json!("path/to+x"));
        assert_eq!(
            body(Value::Object(map)),
            "{\"a\":\"path/to+x\",\"z\":1}\n",
            "sorted, compact, newline-terminated, escaping on"
        );
    }

    #[test]
    fn numbers_and_nulls_are_unchanged_by_the_formatter() {
        assert_eq!(
            body(json!({"i": 75000, "f": 72.5, "n": null, "b": false})),
            "{\"b\":false,\"f\":72.5,\"i\":75000,\"n\":null}\n"
        );
    }
}
