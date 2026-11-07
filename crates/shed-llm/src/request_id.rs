//! The id that goes in the brackets at the start of a transport log line.
//!
//! Ported from `current_request_id` (`agent.py:973`), which is where `call_lm_http_with_retry`
//! gets the `[{rid}]` at 1040 and `discover_lm_studio_models` gets the one at 1546.
//!
//! ## The fact that decided this file's shape
//!
//! The name reads like a lookup, and it is mostly one, but the tail of it is not: when nothing
//! has been set, `current_request_id` **generates** `uuid4().hex` (982) and writes it into the
//! ContextVar (983), so the next call in the same thread gets the same id. A boot that never
//! serves a request therefore logs a generated id, not an empty bracket — measured against the
//! source's own venv rather than read off the plan, which describes 973–1035 as plumbing only.
//!
//! ## Why the clone does not use a context variable
//!
//! The source stores this in `request_id_var` (`agent.py:962`) and reads it from inside functions
//! that never mention it: the retry ladder at 1040, the discovery pass at 1546, the error response
//! at 992, the boundary handler at 1008. That is a second, invisible parameter on every one of
//! those functions, and it is exactly what `How_To_Code` principle 19 is against — a value with no
//! declared source that a reader has to guess at. The Rust port makes it a parameter: every
//! function that prints it takes it, and the caller says where it came from.
//!
//! ## What this file is, and what it is not
//!
//! It is the fallback of 982, which is the half Stage 2 has a use for. The other half of the
//! source — reading `X-Request-ID` off a request (`978`), the `before_request` hook that sets one
//! for every `/api/` path (`1015`), the `after_request` hook that puts it back on the response
//! (`1023`) — arrives at Stage 3 with the routes that have a request to read it from. Writing the
//! ContextVar machinery now would be principle 16 with a fixture attached.

use uuid::Uuid;

/// `agent.py:982`'s `uuid4().hex`: 32 lowercase hex characters, no dashes.
///
/// A caller that has an id already — a served request, from Stage 3 on — passes that instead and
/// never calls this. A caller that has none, which is a boot, calls it once and threads the value
/// through everything that prints it, which is what the ContextVar did in the source.
pub fn new_request_id() -> String {
    Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_id_is_thirty_two_hex_characters_because_that_is_what_uuid4_hex_is() {
        let id = new_request_id();
        assert_eq!(
            id.len(),
            32,
            "uuid4().hex has no dashes, unlike str(uuid4())"
        );
        assert!(
            id.chars().all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "the id is printed in brackets on a log line, so its shape is part of what a reader sees: {id}"
        );
    }

    #[test]
    fn two_generated_ids_are_not_the_same_id() {
        // The source caches one per thread; the clone threads one per flow. Either way, two
        // separate flows must not share an id, because the id is what tells two boot passes apart
        // in a log file that holds both.
        assert_ne!(new_request_id(), new_request_id());
    }
}
