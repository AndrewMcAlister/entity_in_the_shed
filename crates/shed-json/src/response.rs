//! A response that claims to be JSON, checked before anything trusts it.
//!
//! Ported from `utils/json_util.py:43` (`parse_json_response_strict`), which
//! is the guard in front of every LM Studio read: a 200 whose body is an HTML
//! error page, or a JSON body that is cut off mid-write, would otherwise reach
//! the caller as a value with the wrong shape.

use serde_json::Value;

use shed_text::safe_body_snippet;

/// A response that was not the JSON its caller expected.
///
/// The wording is `utils/json_util.py:55-56` and `64-65` verbatim, because
/// these strings are what the source writes into its log and what a captured
/// Python error message reads as: `unit-source returned non-JSON content
/// (status=500, content_type=text/html). Body snippet: …`.
///
/// The two variants differ in one detail the source differs in. The first
/// prints `content_type or 'unknown'`, so a response with no `Content-Type`
/// header says `unknown`; the second prints the header as it stands, which is
/// never empty because an empty header took the first branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    NotJson {
        source_name: String,
        status: u16,
        content_type: String,
        snippet: String,
    },
    InvalidJson {
        source_name: String,
        status: u16,
        content_type: String,
        snippet: String,
    },
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The two arms spell the source's two f-strings, fields in order.
        match self {
            Self::NotJson {
                source_name,
                status,
                content_type,
                snippet,
            } => write!(
                f,
                "{source_name} returned non-JSON content (status={status}, \
                 content_type={content_type}). Body snippet: {snippet}"
            ),
            Self::InvalidJson {
                source_name,
                status,
                content_type,
                snippet,
            } => write!(
                f,
                "{source_name} returned invalid JSON (status={status}, \
                 content_type={content_type}). Body snippet: {snippet}"
            ),
        }
    }
}

impl std::error::Error for JsonError {}

/// Parse a response body as JSON, or explain why it is not.
///
/// The Python helper is handed a `requests.Response` and reaches into it for
/// the three facts it reports. Here those three are the arguments, because the
/// caller that already has the response — the retry ladder in `shed-llm` — is
/// the one that knows its status and headers, and because a response is not
/// something a test can build without a socket.
///
/// `source_name` is the name the message blames, which is
/// `parse_json_response_strict`'s own parameter in the source: `agent.py`
/// passes `"LM Studio model discovery"` (`1559`), `"LM summary response"`
/// (`3980`), `"LM user profile response"` (`4066`) and
/// `"LM Engine chat completion"` (`6788`). `content_type` is the raw
/// `Content-Type` header, which this function lowercases the way
/// `utils/json_util.py:44` does, and `body` is the body text the failure quotes.
///
/// The value is a `serde_json::Value` rather than a map: `response.json()` in
/// Python returns whatever the document is, and the discovery endpoint's
/// document is an object while `parse_discovery_candidates` also accepts a
/// bare list.
pub fn parse_json_response_strict(
    source_name: &str,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<Value, JsonError> {
    let content_type = content_type.to_lowercase();

    if !content_type.contains("application/json") {
        let shown = if content_type.is_empty() {
            "unknown"
        } else {
            &content_type
        };
        return Err(JsonError::NotJson {
            source_name: source_name.to_string(),
            status,
            content_type: shown.to_string(),
            snippet: safe_body_snippet(body),
        });
    }

    serde_json::from_str(body).map_err(|_| JsonError::InvalidJson {
        source_name: source_name.to_string(),
        status,
        content_type,
        snippet: safe_body_snippet(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The body that was accepted, which is what `test_parses_valid_json`
    /// asserts on.
    fn parsed(content_type: &str, body: &str) -> Value {
        parse_json_response_strict("unit-source", 200, content_type, body)
            .unwrap_or_else(|err| panic!("a {content_type} body should have parsed: {err}"))
    }

    /// The message a rejected response produced, which is what the source's
    /// tests assert on.
    fn message(status: u16, content_type: &str, body: &str) -> String {
        match parse_json_response_strict("unit-source", status, content_type, body) {
            Ok(value) => panic!("a {content_type} body should not have parsed: {value}"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn a_json_body_is_returned_as_a_value() {
        assert_eq!(parsed("application/json", "{\"ok\": 1}"), json!({"ok": 1}));
    }

    #[test]
    fn a_json_body_that_is_a_list_is_accepted() {
        // `response.json()` returns whatever the document is, and the discovery
        // parser accepts a bare list as well as an object.
        assert_eq!(parsed("application/json", "[1, 2]"), json!([1, 2]));
    }

    #[test]
    fn the_header_is_matched_case_insensitively_and_kept_as_it_was_given() {
        // `"application/json" in content_type` is a substring test, so a charset
        // is not a problem; the message quotes the lowercased header, which is
        // what the source reports.
        assert_eq!(parsed("Application/JSON", "{\"ok\": 1}"), json!({"ok": 1}));
        assert_eq!(
            message(200, "application/json; charset=UTF-8", "{oops"),
            "unit-source returned invalid JSON (status=200, \
             content_type=application/json; charset=utf-8). Body snippet: {oops"
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_reported_with_the_body_that_came_back() {
        assert_eq!(
            message(200, "text/html", "not json"),
            "unit-source returned non-JSON content (status=200, content_type=text/html). \
             Body snippet: not json"
        );
    }

    #[test]
    fn a_response_with_no_content_type_says_unknown() {
        assert_eq!(
            message(500, "", "boom"),
            "unit-source returned non-JSON content (status=500, content_type=unknown). \
             Body snippet: boom"
        );
    }

    #[test]
    fn a_valid_header_with_an_unparsable_body_says_invalid_json() {
        assert_eq!(
            message(200, "application/json", "{\"ok\": "),
            "unit-source returned invalid JSON (status=200, content_type=application/json). \
             Body snippet: {\"ok\":"
        );
    }

    #[test]
    fn the_snippet_is_one_line_and_cut_at_three_hundred_characters() {
        let body = format!("{}\n{}", "problem ", "z".repeat(400));
        let message = message(200, "text/plain", &body);
        assert!(message.contains("Body snippet: problem "), "{message}");
        assert!(message.ends_with("..."), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }
}
