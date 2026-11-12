# Slice 4a brief — conversation records, identity and file names

Hand this file to the session as its task. It is complete on its own: the source is pasted
below, the decisions are already made, and the stop condition is stated. **Do not read
`agent.py` for this slice.** Do not read `conversation_history.py` beyond line 216 — the rest
is slices 4b and 4c.

- Reference app: `D:\Entity-In-The-Shed` (hyphen). Clone: `D:\Entity_In_The_Shed` (underscore).
- Plan: `TODO/PLAN-Rust-Clone.md` §8.2 (this is slice 4a) and §3.2/§3.3 (crate map, layers).
- Rules: `How_To_Code.md`. Workspace lints deny `dead_code`, `unused`, `unused_crate_dependencies`,
  `clippy::unwrap_used`, `clippy::expect_used` **including test targets**.

## Mission

Create `crates/shed-conversation` and port the pure identity layer: the record types, address
normalisation, identity/file naming, and the message-window calculation. It is the crate that
gives `shed-store::normalize_client_id` its first production caller — the reason part 1 landed
first. No I/O, no `agent.py`, no HTTP.

## The source, verbatim (`utils/conversation_history.py`, lines 42–216)

The module docstring states the two rules this slice implements. Condensed:

> A conversation belongs to the browser, identified by the id it generated once and stored —
> `client-<id>.json` — so the same person coming back continues the exchange and two computers
> behind one router do not share one transcript. The address is the fallback for a client that
> sends no id, stored as itself — `192.168.1.34.json`. Neither form depends on the time or on
> chance: the same request always names the same file. […] The exact conversation is bounded by
> *messages* (the bubbles the person saw); a reply stored as one row per beat costs several.
> Ten rows is the promise because the prompt is what has to stay small. […] The record also keeps
> a `user_details` profile, saved as each turn ends; `other` is a reserved catch-all that may run
> to 512 characters. `user_details_covered_through` names the newest message the profile has read.

```python
class ConversationMessage(TypedDict):
    """One row of the exact transcript: what was said, and when."""
    role: str
    content: str
    timestamp: str

class ConversationSummary(TypedDict):
    """One block of remembered material, covering a span already trimmed."""
    start_timestamp: str
    end_timestamp: str
    summary: str

class ConversationRecord(TypedDict):
    """A conversation as stored: the exact tail and the memory behind it."""
    messages: list[ConversationMessage]
    summaries: list[ConversationSummary]
    # Optional because records written before this field existed carry none.
    delivery_style: NotRequired[str]
    # Optional for the same reason; the first idle pass fills it in.
    user_details: NotRequired[dict[str, str]]
    # A timestamp rather than a count, because trimming removes rows from the front.
    user_details_covered_through: NotRequired[str]

class ConversationCompactionPlan(NamedTuple):
    """What one housekeeping pass should do with a record.

    ``consumed_count`` names the prefix of the stored messages this plan accounts for. It is
    carried rather than recomputed so a turn that lands while the summary is being written is
    not dropped along with it: the housekeeper removes exactly the rows the plan read.
    """
    messages_to_summarize: list[ConversationMessage]
    retained_messages: list[ConversationMessage]
    retained_summaries: list[ConversationSummary]
    consumed_count: int

def _normalized_ip(value) -> str:
    """The canonical spelling of one address, or ``""`` when it is not one.

    A scope id (``fe80::1%12``) is stripped first: it names the interface the address was seen
    on, not the client, and the stdlib parser rejects it.
    """
    text = str(value or "").strip()
    if not text:
        return ""
    if text.startswith("[") and text.endswith("]"):
        text = text[1:-1]
    text = text.split("%", 1)[0]
    try:
        parsed = ip_address(text)
    except ValueError:
        return ""
    mapped = getattr(parsed, "ipv4_mapped", None)
    if mapped is not None:
        return str(mapped)
    return str(parsed)

def resolve_client_ip(cf_connecting_ip, forwarded_for, remote_addr) -> str:
    """The client's address from the headers a proxied request carries.

    Cloudflare's own header is the most specific statement of who called, then the first entry
    of ``X-Forwarded-For`` (the original client), then the socket peer. Behind the tunnel the
    peer is the local ``cloudflared`` process, so it is the last resort rather than the answer.
    """
    forwarded = str(forwarded_for or "").split(",", 1)[0]
    for candidate in (cf_connecting_ip, forwarded, remote_addr):
        resolved = _normalized_ip(candidate)
        if resolved:
            return resolved
    return ""

CONVERSATION_CLIENT_PREFIX = "client-"

def conversation_client_id(conversation_key) -> str:
    """The browser id a conversation key names, or ``""``.

    Re-reading the id rather than trusting the string is what keeps an unvalidated value from
    reaching a path.
    """
    text = str(conversation_key or "").strip()
    if not text.startswith(CONVERSATION_CLIENT_PREFIX):
        return ""
    return normalize_client_id(text[len(CONVERSATION_CLIENT_PREFIX):])

def conversation_identity(client_id, client_ip) -> str:
    """The key a conversation is stored under: the browser's id, else its address.

    The id is generated once by the page and sent on every turn, so the same browser always
    resolves to the same key -- an address cannot, because every computer behind one home
    router shares it. The server-minted replacement is deliberately not used here: a client
    that never stores it would otherwise start a new conversation on every request.
    """
    client = normalize_client_id(client_id)
    if client:
        return f"{CONVERSATION_CLIENT_PREFIX}{client}"
    return _normalized_ip(client_ip)

def conversation_filename(conversation_key) -> str:
    """The file name a conversation key is stored under.

    Only a validated browser id or an address can become a name. Rejecting anything else is
    what keeps a header value out of the path: it is attacker-controlled text on a request
    that arrives from the internet.
    """
    resolved = _normalized_ip(conversation_key)
    if resolved:
        return f"{resolved.replace(':', '-')}.json"
    client = conversation_client_id(conversation_key)
    if client:
        return f"{CONVERSATION_CLIENT_PREFIX}{client}.json"
    raise ValueError(f"conversation key names neither an address nor a browser: {conversation_key!r}")

def names_a_conversation(filename) -> bool:
    """Whether a stored file name is a conversation.

    Old records are named for the sha1 of a session, and folders also hold the character
    outline. The housekeeper must not spend a model call summarizing either, and retention must
    not treat the outline as a conversation.
    """
    name = str(filename or "")
    if not name.endswith(".json"):
        return False
    stem = name[: -len(".json")]
    if conversation_client_id(stem):
        return True
    # An IPv6 address is stored with dashes, so both spellings are tried.
    for candidate in (stem, stem.replace("-", ":")):
        try:
            if conversation_filename(candidate) == name:
                return True
        except ValueError:
            continue
    return False

def message_window_start_index(messages, max_messages) -> int:
    """Where the newest ``max_messages`` rows begin.

    The window is counted in messages -- the bubbles the person saw -- because the point of the
    cap is the size of the prompt. ``0`` means everything fits; a cap of zero trims all.
    """
    if max_messages <= 0:
        return len(messages)
    return max(0, len(messages) - int(max_messages))
```

## Decisions already taken — do not re-derive

1. **Types.** `ConversationMessage`, `ConversationSummary`, `ConversationRecord`,
   `ConversationCompactionPlan` become `serde` structs. `ConversationRecord`'s three optional
   fields are `Option<T>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` —
   a record written before they existed must load, and must be written back **absent, not
   null**. Pin that with a round-trip test; it is the one thing the Python `NotRequired`
   subtlety can silently get wrong. `user_details` is `BTreeMap<String, String>` for now —
   **4c owns ordering** (Python dicts are insertion-ordered and the prompt may render them) and
   may replace it; record that decision then, do not guess now.
2. **Addresses.** `_normalized_ip` → `normalize_client_ip(value: Option<&str>) -> Option<String>`.
   Order: trim → strip one leading `[`/trailing `]` pair only when **both** are present → cut at
   the first `%` → `IpAddr::from_str` → if v6 with `to_ipv4_mapped`, render the v4 address →
   else render the parsed address. Return `None` where Python returns `""`.
   Python's `str(ip)` and Rust's `Display` for `Ipv6Addr` are both RFC 5952 compressed
   lowercase; the ported tests decide whether any case differs.
3. **`resolve_client_ip`** takes three `Option<&str>` and returns `Option<String>`: split
   `forwarded_for` at the first `,` first, then try Cloudflare header → that first entry → peer.
4. **`conversation_client_id`** → `conversation_client_id(key: Option<&str>) -> Option<String>`:
   trim, require the `client-` prefix, then delegate to `shed_store::normalize_client_id`
   (which returns `Option<String>`); a malformed remainder is `None`, never an error.
5. **`conversation_filename`** is the only fallible function:
   `conversation_filename(key: &str) -> Result<String, ConversationKeyError>`. The error's
   `Display` reproduces Python's message shape, quoting the key. **Keep it a rejection** — it is
   the guard that keeps a header value out of a path; do not sanitise.
6. **`names_a_conversation`** → `names_a_conversation(filename: Option<&str>) -> bool`. Try the
   stem as given, then with every `-` turned into `:`, and compare against the full name
   including `.json`.
7. **`message_window_start_index`** → `message_window_start_index(len: usize, max_messages: i64) -> usize`.
   Python's `int(max_messages)` truncates a float; the Rust callers pass integers, so take `i64`
   and say so in a comment. `max_messages <= 0` returns `len`; otherwise
   `len.saturating_sub(usize::try_from(max_messages).unwrap_or(usize::MAX))`.
8. **Crate skeleton.** `crates/shed-conversation/Cargo.toml` with `[dependencies]` naming only
   what is used — `shed-store` (required: the id normaliser) and `serde` with `derive`;
   `serde_json` under `[dev-dependencies]` for the round-trip test. Add `shed-config`, `shed-json`,
   `shed-text`, `shed-models` or `shed-prompts` **only** when a later slice uses them —
   `unused_crate_dependencies` is denied and will fail the gate otherwise. `rustfmt` is
   `max_width = 100`; run `cargo fmt --all` after writing.
9. **The 31 tests to port** (from `tests/test_conversation_history_utils.py`, all in this slice):
   - `_normalized_ip`/`resolve_client_ip` (7): `the_cloudflare_header_is_the_most_specific_statement`,
     `the_first_forwarded_entry_is_the_original_client`, `the_socket_peer_is_the_last_resort`,
     `a_malformed_header_falls_through_to_the_next_candidate`,
     `nothing_usable_is_an_empty_name_rather_than_an_error`,
     `an_ipv4_mapped_address_is_stored_as_ipv4`, `a_scope_id_is_stripped_before_parsing`
   - `conversation_identity` (4): `the_browser_id_is_preferred_over_the_address`,
     `the_address_is_the_fallback_when_no_valid_id_is_sent`,
     `the_same_request_always_names_the_same_key`, `no_identifier_at_all_names_nothing`
   - `conversation_filename` (7): `an_ipv4_address_names_the_file_itself`,
     `an_ipv6_address_uses_dashes_because_colons_are_not_portable`,
     `a_mapped_address_names_the_same_file_as_its_ipv4_form`,
     `a_browser_id_names_a_file_of_its_own`, `two_browsers_at_one_address_name_different_files`,
     `a_header_that_is_not_an_address_is_refused`, `a_malformed_browser_id_is_refused`,
     `an_empty_key_is_refused` — that is 8 names; port all 8
   - `names_a_conversation` (7): `an_ipv4_record_is_one`, `an_ipv6_record_is_one`,
     `a_browser_record_is_one`, `a_malformed_browser_record_is_not`,
     `a_legacy_session_record_is_not`, `the_character_outline_is_not`, `a_backup_is_not`
   - `message_window_start_index` (5): `the_window_is_the_newest_rows`,
     `every_reply_beat_costs_one_of_the_window`, `fewer_messages_than_the_cap_keeps_everything`,
     `an_empty_transcript_keeps_everything`, `a_cap_of_zero_trims_everything`
   - plus the `ConversationRecord` round-trip test described in decision 1.
   Place them as a `#[cfg(test)] mod tests` per module and one `tests/records.rs` for the
   round-trip. Read the Python test file for the cases' bodies **only** if a name is ambiguous —
   it is 518 lines and this list is the map.

## Standing rules for this repo (from the resume notes — they are not optional)

- **Gate runs:** `cmd /c "cargo ... 2>&1"`. PowerShell turns cargo's stderr into
  `NativeCommandError` noise and can leave `$LASTEXITCODE = 1` on a passing run.
- **PowerShell:** `;` not `&&`. Do not use `Get-Content f | Measure-Object -Line` to count lines
  — it drops blank lines. .NET APIs ignore PowerShell's current directory: pass absolute paths.
- **Run the gate before committing, not after.** Part 1 arrived with fmt and clippy red.

## Gate (must be green before the commit)

```
cargo fmt --all -- --check
cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
cmd /c "cargo test --workspace 2>&1"        # 360 before this slice; 360 + 32 after
cmd /c "npm test 2>&1"                      # 250, unchanged
```

Update `docs/PARITY.md` only if the slice diverges from the source (the absent-field rule and
the IPv6 rendering are the two candidates). No guard catalogue entry: none of this is a
guard — it is a pure rule.

## Stop condition — read this last, obey it exactly

The slice ends one of two ways, and nothing else is in scope:

1. **Gate green** → commit
   `Stage 4 (part 2a): shed-conversation records, identity and file names`, push, append one
   line to `TODO/RESUME-NOTES.md` under "Where we are", and **stop**. Do not start slice 4b.
2. **Gate red after two honest fix attempts**, or the source and this brief disagree → **stop**
   and report the failing output plus the disagreement. Do not widen the slice, do not read
   `agent.py`, do not "fix it later".
