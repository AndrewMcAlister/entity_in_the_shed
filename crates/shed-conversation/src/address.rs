//! The address a request came from, once.
//!
//! Ported from `utils/conversation_history.py:93-129`. The address is the
//! fallback identity for a browser that sends no id, so it is worked out here
//! from what a proxied request carries rather than from the socket alone: behind
//! the tunnel the peer is the local `cloudflared` process, not the person.
//!
//! Both functions answer `None` where the source answers `""`. That is the answer
//! meaning "no usable identity" — callers already read an empty key as "no
//! memory", so a request that cannot be attributed loses its memory rather than
//! its turn (`nothing_usable_is_an_empty_name_rather_than_an_error`).

use std::net::IpAddr;

use crate::py_strip;

/// The canonical spelling of one address, or `None` when it is not one.
///
/// A scope id (`fe80::1%12`) is stripped first: it names the interface the
/// address was seen on, not the client, and the parser rejects it.
///
/// An IPv4-mapped v6 address (`::ffff:192.168.1.34`) is stored as the v4 address
/// inside it, so a client that reaches the app over v6 and one that reaches it
/// over v4 name the same conversation when they are the same machine.
///
/// `str(value or "").strip()` is [`py_strip`] over `unwrap_or("")`: the falsy
/// reading belongs to a JSON value and lives in `shed_text::text_or_empty`, which
/// the layer map lets this crate reach but which this function does not need
/// until a route hands it a body field (slice 4b).
#[must_use]
pub fn normalize_client_ip(value: Option<&str>) -> Option<String> {
    let mut text = py_strip(value.unwrap_or(""));
    if text.is_empty() {
        return None;
    }
    // `[::1]` is how a URL spells a v6 address; brackets are stripped only as a
    // matched pair, so a stray one on either side stays and fails the parse.
    if let Some(bracketed) = text
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
    {
        text = bracketed;
    }
    text = text.split('%').next().unwrap_or(text);
    let parsed: IpAddr = text.parse().ok()?;
    if let IpAddr::V6(six) = parsed {
        if let Some(four) = six.to_ipv4_mapped() {
            return Some(four.to_string());
        }
    }
    Some(parsed.to_string())
}

/// The client's address from the headers a proxied request carries.
///
/// Cloudflare's own header is the most specific statement of who called, then the
/// first entry of `X-Forwarded-For` (the original client), then the socket peer.
/// The first entry is taken before anything is parsed, and the entry after a
/// comma is tried too, because a chain of proxies puts its own hops in the same
/// header — and a hop's address is a private one, which is not an identity.
///
/// `forwarded_for` is split at the first `,` before the candidates are tried, as
/// the source splits it; the split is not a validation step, so a header that is
/// only a list of proxies still falls through to the peer.
#[must_use]
pub fn resolve_client_ip(
    cf_connecting_ip: Option<&str>,
    forwarded_for: Option<&str>,
    remote_addr: Option<&str>,
) -> Option<String> {
    let forwarded = forwarded_for.unwrap_or("").split(',').next();
    [cf_connecting_ip, forwarded, remote_addr]
        .into_iter()
        .find_map(normalize_client_ip)
}

#[cfg(test)]
mod tests {
    use super::resolve_client_ip;

    /// `ResolveClientIpTests` (`test_conversation_history_utils.py:69-101`).
    ///
    /// The source passes `""` for a header the request did not carry; the Rust
    /// signature spells that `None`, and both are exercised below because they are
    /// the same falsy case in Python (`str(value or "")`).

    #[test]
    fn the_cloudflare_header_is_the_most_specific_statement() {
        let resolved = resolve_client_ip(
            Some("203.0.113.7"),
            Some("198.51.100.2, 10.0.0.1"),
            Some("127.0.0.1"),
        );

        assert_eq!(resolved.as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn the_first_forwarded_entry_is_the_original_client() {
        let resolved = resolve_client_ip(None, Some("198.51.100.2, 10.0.0.1"), Some("127.0.0.1"));

        assert_eq!(resolved.as_deref(), Some("198.51.100.2"));
    }

    #[test]
    fn the_socket_peer_is_the_last_resort() {
        // Behind the tunnel the peer is the local `cloudflared`, so it is the last
        // resort rather than the answer — but it is still an address.
        assert_eq!(
            resolve_client_ip(None, None, Some("192.168.1.34")).as_deref(),
            Some("192.168.1.34")
        );
    }

    #[test]
    fn a_malformed_header_falls_through_to_the_next_candidate() {
        let resolved = resolve_client_ip(
            Some("not-an-ip"),
            Some("198.51.100.2"),
            Some("192.168.1.34"),
        );

        assert_eq!(resolved.as_deref(), Some("198.51.100.2"));
    }

    #[test]
    fn nothing_usable_is_an_empty_name_rather_than_an_error() {
        // Callers already read an empty key as "no memory"; a request that
        // cannot be attributed should lose memory, not the turn.
        assert_eq!(resolve_client_ip(Some("unknown"), Some(""), None), None);
    }

    #[test]
    fn an_ipv4_mapped_address_is_stored_as_ipv4() {
        assert_eq!(
            resolve_client_ip(Some("::ffff:192.168.1.34"), None, None).as_deref(),
            Some("192.168.1.34")
        );
    }

    #[test]
    fn a_scope_id_is_stripped_before_parsing() {
        // `fe80::1%12` is what a link-local peer reports; the interface is not
        // part of the identity, and the stdlib parser rejects the suffix.
        assert_eq!(
            resolve_client_ip(Some("fe80::1%12"), None, None).as_deref(),
            Some("fe80::1")
        );
    }
}
