#!/usr/bin/env python3
"""Capture the D8 golden fixtures for `crates/shed-chat` (slice 4f-1).

Run from anywhere; paths are absolute. Nothing here boots the reference app:

    python tools\\capture_reply_fixtures.py

`utils/reply.py` is imported — it imports `json`, `re` and `typing` and nothing
else, so importing it starts no server and reads no file. The `agent.py` symbols
are **not** imported: `agent.py` is an 11,607-line application module and
importing it would run the app. The brief's rule is that the pure range is read
out of the file and `exec`'d in a prepared namespace, which is what
`:func:`load_agent_range`` does. That is still "the pure Python module in a
subprocess", and it boots nothing.

Five names the pasted range reads but does not define. The free-name check in
``:func:`load_agent_range``` is what found them: it refuses to exec the range
until the namespace supplies every global the range reads, so a missed
dependency is a message that names it rather than a ``NameError`` halfway
through a capture.

* ``PROTECTED_TEXT_PATTERN`` — ``utils/text.py:294``. Imported from there
  (``utils/text.py`` imports only ``re``).
* three functions out of ``utils/reply.py`` — ``extract_reply_segments``,
  ``strip_tool_call_envelope`` and ``tool_calls_in_envelope``, called by
  ``settle_prose_tool_calls`` and by ``ReplyStreamPublisher``. ``agent.py``
  imports the *module*, so they arrive as globals of the range. In the Rust port
  they are the same crate's ``quality`` module: this adds no dependency edge,
  only a ``use`` inside ``shed-chat``.
* ``COMMON_NON_TERMINAL_ABBREVIATIONS`` — ``agent.py:283``, *outside* the pasted
  5504-6202 range, and read by two functions inside it
  (``normalize_reply_text`` and ``SentenceSplitter._is_non_terminal_period``).
  ``ast.parse`` + ``ast.literal_eval`` lift the one assignment out of the file
  text: the module is never imported, never exec'd, and no name in it runs. The
  value is written into ``splitter.json`` as ``common_non_terminal_abbreviations``
  so the Rust port reads a captured constant rather than a remembered one.

The script is re-runnable and its output is committed. A reader who doubts that
the brief's pasted source is the reference app's source runs this and gets a
diff, or does not.
"""

from __future__ import annotations

import ast
import builtins
import collections
import json
import random
import re
import symtable
import sys
import threading
from pathlib import Path
from typing import Any

REFERENCE_APP = Path(r"D:\Entity-In-The-Shed")
AGENT_PY = REFERENCE_APP / "agent.py"
FIXTURE_DIR = Path(__file__).resolve().parent.parent / "crates" / "shed-chat" / "tests" / "fixtures"

# `utils` is a package inside the reference app whose `__init__.py` is a docstring, so importing
# `utils.reply` starts no server and reads no file. Nothing here imports `agent.py`.
if str(REFERENCE_APP) not in sys.path:
    sys.path.insert(0, str(REFERENCE_APP))

# The one range the brief pastes, 1-based and inclusive.
RANGE_START = 5504
RANGE_END = 6202


# --------------------------------------------------------------------------
# the pure Python, without importing the application
# --------------------------------------------------------------------------
def load_agent_range(namespace: dict[str, Any]) -> dict[str, Any]:
    """``exec`` agent.py:5504-6202 in ``namespace`` and return its public names."""
    lines = AGENT_PY.read_text(encoding="utf-8").splitlines()
    body = "\n".join(lines[RANGE_START - 1 : RANGE_END])
    if len(lines) < RANGE_END:
        raise SystemExit(f"{AGENT_PY} has {len(lines)} lines; cannot read {RANGE_START}-{RANGE_END}")
    # Fail loudly on the first line of the range: it must be a def/class, not the middle of one.
    first = lines[RANGE_START - 1]
    if not (first.startswith("def ") or first.startswith("class ") or first.startswith("@")):
        raise SystemExit(f"{AGENT_PY}:{RANGE_START} is not the start of a symbol: {first!r}")

    code = compile(body, f"agent.py:{RANGE_START}-{RANGE_END}", "exec")
    missing = sorted(_reads_from_namespace(body, f"agent.py:{RANGE_START}-{RANGE_END}") - set(namespace))
    if missing:
        raise SystemExit(
            "the pasted range reads names the prepared namespace does not supply: "
            + ", ".join(missing)
            + "\nAdd them to `agent_namespace()` — the range must exec with no app behind it."
        )
    exec(code, namespace)  # noqa: S102 - the whole point: the pure range, and nothing else
    return namespace


def _reads_from_namespace(body: str, filename: str) -> set[str]:
    """Names the range reads from its enclosing globals: what the namespace must supply.

    ``symtable`` answers this exactly, which matters: an ``ast`` walk over-approximates and
    reports every function parameter (``self``, ``chunk``) as an unresolved name, and a check
    that always fails says nothing. A symbol is a read of an enclosing global when it is
    referenced and global from its own scope without being bound there; the range's own
    module-level assignments and the builtins (which ``exec`` supplies) are then subtracted.
    """
    top = symtable.symtable(body, filename, "exec")
    own = {
        symbol.get_name()
        for symbol in top.get_symbols()
        if symbol.is_assigned() or symbol.is_imported()
    }
    reads: set[str] = set()

    def walk(scope: symtable.Scope) -> None:
        for symbol in scope.get_symbols():
            # `Symbol.get_type()` is 3.13+; this box runs 3.12, so the predicates say it instead.
            # A read of an enclosing global: referenced here, global from here, and not something
            # this scope itself binds (that is a local, or a module-level name the range defines).
            if (
                symbol.is_referenced()
                and symbol.is_global()
                and not symbol.is_assigned()
                and not symbol.is_imported()
                and not symbol.is_parameter()
            ):
                reads.add(symbol.get_name())
        for child in scope.get_children():
            walk(child)

    walk(top)
    return reads - own - set(dir(builtins))


class _RecordingLogger:
    """Stands in for ``app.logger``: ``settle_prose_tool_calls`` warns through it."""

    def __init__(self) -> None:
        self.warnings: list[tuple[str, tuple[Any, ...]]] = []

    def warning(self, message: str, *args: Any) -> None:
        self.warnings.append((message, args))


class _App:
    def __init__(self) -> None:
        self.logger = _RecordingLogger()


def agent_namespace() -> dict[str, Any]:
    """Everything the pasted range touches, and nothing the app would need."""
    from utils.text import PROTECTED_TEXT_PATTERN  # imports `re` only

    namespace: dict[str, Any] = {
        "json": json,
        "re": re,
        "ast": ast,
        "app": _App(),
        "PROTECTED_TEXT_PATTERN": PROTECTED_TEXT_PATTERN,
        "COMMON_NON_TERMINAL_ABBREVIATIONS": common_non_terminal_abbreviations(),
        "__name__": "shed_chat_capture",
    }
    # The range calls these three out of `utils/reply.py`: `agent.py` imports the module rather
    # than the names, so they are among the range's globals. In the Rust port they are the same
    # crate's `quality` module, which is why this adds no dependency edge.
    from utils import reply as reply_module

    for name in (
        "extract_reply_segments",
        "strip_tool_call_envelope",
        "tool_calls_in_envelope",
    ):
        namespace[name] = getattr(reply_module, name)
    return namespace


def common_non_terminal_abbreviations() -> frozenset[str]:
    """``agent.py:283``'s set, read as data: the module is never imported."""
    tree = ast.parse(AGENT_PY.read_text(encoding="utf-8"), filename=str(AGENT_PY))
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "COMMON_NON_TERMINAL_ABBREVIATIONS":
                return frozenset(ast.literal_eval(node.value))
    raise SystemExit("COMMON_NON_TERMINAL_ABBREVIATIONS not found at module level in agent.py")


def write_fixture(name: str, payload: Any) -> None:
    FIXTURE_DIR.mkdir(parents=True, exist_ok=True)
    path = FIXTURE_DIR / name
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"  wrote {path.relative_to(FIXTURE_DIR.parent.parent)}")


# --------------------------------------------------------------------------
# chunk sequences: whole, one character at a time, two seeded splits
# --------------------------------------------------------------------------
def seeded_chunks(text: str, seed: int, pieces: int) -> list[str]:
    """`pieces` pieces cut at `pieces - 1` distinct internal boundaries, from `seed`.

    Arbitrary on purpose, deterministic on purpose too: a reader who runs this
    script gets the same chunks, so a fixture that pins a split across an escape
    pins it the same way forever.
    """
    rng = random.Random(seed)
    if len(text) < 2:
        return [text]
    cuts = sorted(rng.sample(range(1, len(text)), min(pieces - 1, len(text) - 1)))
    bounds = [0, *cuts, len(text)]
    return [text[a:b] for a, b in zip(bounds, bounds[1:]) if text[a:b]]


def chunk_sequences(text: str) -> list[dict[str, Any]]:
    """The four ways a document can arrive, per the brief."""
    out: list[dict[str, Any]] = [
        {"label": "whole", "chunks": [text]},
        {"label": "one-character", "chunks": list(text)},
    ]
    for seed, pieces in ((20261120, 3), (47424, 5)):
        out.append({"label": f"seeded-{seed}", "chunks": seeded_chunks(text, seed, pieces)})
    return out


# --------------------------------------------------------------------------
# reply_text
# --------------------------------------------------------------------------
NORMALIZE_INPUTS: list[Any] = [
    "",
    None,
    "Hello there.",
    "literal backslash n:\\nsecond line",
    "literal crlf:\\r\\nsecond line",
    "real\r\nnewline\nand one",
    "one\n\n\n\n\nfour in a row",
    "trailing spaces   ",
    "Mr. Smith said hello.Did he agree?He did.",
    "Wait,hold on;then,go",
    "Dr. Jones and Mr. Smith left.Both of them.",
    "See https://example.com, and after it.",
    "Mail me at a.b+tag@example.com,please.",
    "www.example.com/some/path, next",
    "e.g. this, and i.e. that",
    "A sentence.  Two spaces kept?No.Say it.",
    "U.S. markets rose.IR38a is fine.",
    '{"reply": "nested \\n escapes"}',
    "tabs\tand  double  spaces after a stop.Next.",
    "end with three dots...",
    "3.14 is a number.Ends here.",
    "nothing to normalise, just words",
    "  leading and trailing\n",
    "Quote after stop.\"Like this\",and on.",
    "Version 2. It works.",
]

MARKDOWN_INPUTS: list[str] = [
    "",
    "plain text",
    "**strong** words",
    "*moderate* words",
    "_softer_ words",
    "**a** and *b* and _c_",
    "*not closed",
    "**a*b*c**",
    "snake_case_word inside",
    "**nested _soft_** together",
    "*one* *two*",
    "no markup at all.",
]


def capture_reply_text(agent: dict[str, Any]) -> None:
    write_fixture(
        "reply_text.json",
        {
            "normalize_reply_text": [
                {"input": value, "expected": agent["normalize_reply_text"](value)}
                for value in NORMALIZE_INPUTS
            ],
            "markdown_to_ssml": [
                {"input": value, "expected": agent["markdown_to_ssml"](value)}
                for value in MARKDOWN_INPUTS
            ],
        },
    )


# --------------------------------------------------------------------------
# quality (utils/reply.py)
# --------------------------------------------------------------------------
TEASER_TEXTS: list[Any] = [
    "Let me fetch that.",
    "I'll check on it.",
    "Fetching the data now.",
    "Checking...",
    "Let me take a closer look.",
    "Let me have a look.",
    "Let me look into that.",
    "Let me see what I can find.",
    "I'll take a closer look.",
    "I will have another look.",
    "Taking a closer look now.",
    "Looking into this now.",
    "Let me dig into the numbers.",
    "Let me investigate.",
    "I'll go over it.",
    "Let me know if you need anything else.",
    "Let me know how you get on.",
    "I'll let you know when it is done.",
    "Here is the data.",
    "The weather is nice today.",
    "",
    None,
    "  ",
    "review",
    "I shall retrieve the ledger.",
    "we are looking into it, said the keeper",
]

TEASER_LONG = (
    "The position is up two point four percent on the day and the margin "
    "sits at thirty percent, with the stop still where we left it, so I am "
    "checking nothing further and would leave it be for now, as the trend "
    "has held across every session this week and there is no reason to act."
)

TRADER_TEXTS: list[Any] = [
    "show my mctrader balance",
    "check my broker position",
    "start trader now",
    "stop trader",
    "restart trader please",
    "what is mctrader",
    "check my position",
    "tell me about the watchlist",
    "my portfolio needs attention",
    "analyse the ticker",
    "analyze my symbols",
    "how do I exchange a file",
    "",
    None,
]

INCOMPLETE_CASES: list[tuple[Any, Any]] = [
    ("", None),
    (None, None),
    ("Any text at all", {"_meta": {"done_reason": "length"}}),
    ("Any text at all", {"_meta": {"done_reason": "LENGTH"}}),
    ("Any text at all", {"_meta": {"done_reason": " length "}}),
    ("Any text at all", {"_meta": {}}),
    ("Any text at all", {}),
    ("Any text at all", {"_meta": {"done_reason": None}}),
    ("Done here.", {"_meta": {"done_reason": "stop"}}),
    ("Wait for it:", None),
    ("Hold on...", None),
    ("This is a fairly long incomplete line\nand another line here now", None),
    ("Here's a poem for you", None),
    ("Here is the poem", None),
    ("Here's a short poem about the rain, which I wrote for you just now", None),
    ("  ", None),
    ("The quick brown fox jumped.", None),
    ("A line of text that is long enough to pass the sixty character test", None),
    ("Short.", None),
    ("Two lines\nending with a stop.", None),
]

LATEST_USER_CASES: list[Any] = [
    [
        {"role": "user", "content": "  hi there  "},
        {"role": "assistant", "content": "x"},
        {"role": "user", "content": "two"},
    ],
    [{"role": "assistant", "content": "x"}],
    [],
    None,
    [{"role": "user"}],
    [{"role": "user", "content": 7}],
    [{"role": "user", "content": None}, {"role": "user", "content": " last "}],
]

TASK_STUB_CASES: list[tuple[Any, Any]] = [
    ("write a poem for me", "I will write one for you"),
    ("create a song for me", "Here is a line for you"),
    ("what time is it", "It is 3pm."),
    ("write a poem for me", "I will write one\nfor you"),
    (
        "write a poem for me",
        "I will write one for you right now with some rhymes and verses that flow really nicely today indeed",
    ),
    ("write a poem for me", 'I will write "a poem" for you'),
    ("", "I will"),
    ("write a poem for me", ""),
    ("draft a letter to the bank", "Sure"),
    ("make me a list of tools", "Alright"),
    ("compose a speech", "I shall draft it now"),
    ("write a poem", "The rain came down all night."),
    (None, "I will"),
    ("write a poem", None),
]

MERGE_CASES: list[tuple[Any, Any]] = [
    ("Hello", "Hello world"),
    ("abc", "abc def"),
    ("Hello world", "world is big"),
    ("He said", "said hello"),
    ("Hi", "There you go"),
    ("The end is", "near"),
    ("  partial  ", ""),
    ("", "Done."),
    (None, "Done."),
    ("abc", "abc"),
    ("abc", None),
    ("tail end", "end"),
    ("one two three", "three four five"),
    ("stop.", ".carry on"),
    ("word", "!next"),
    ("a" * 300, ("a" * 250) + " tail"),
]

SEGMENT_VALUES: list[Any] = [
    None,
    [],
    {},
    ["Only this."],
    ["  first  ", "", "   ", "second"],
    ["real", None, 7, {"a": 1}, ["nested"]],
    [f"segment {index}" for index in range(7)],
    "Just the one.",
    ("a", "b"),
    ["Hello,world"],
    "",
    7,
    True,
    ["  x  "] * 10,
]

UNUSABLE_TEXTS: list[Any] = [
    "I could not generate a response",
    "i couldn't generate a response",
    "I am unable to generate a response.",
    "  I cannot generate a response!  ",
    "Unable to generate a response",
    "I'm unable to generate a response",
    "i am unable to generate a response",
    "Hello there",
    "",
    None,
    "   ",
    '"I could not generate a response"',
    "I could not generate a response today",
]

OUT_OF_CHARACTER_TEXTS: list[Any] = [
    "As an AI, I think it is fine.",
    "I do not possess a personal identity.",
    "as an AI assistant, no",
    "I don't possess a personal identity",
    "The weather is nice.",
    "",
    None,
]

RECOVERY_META_TEXTS: list[Any] = [
    "There was no content in the previous reply.",
    "empty object",
    "There was no reply text to continue.",
    "Only an empty reply field in the JSON.",
    "system returned an empty object",
    "I did not intend to cut you short.",
    "All done here.",
    "",
    None,
    "  ",
    "there was no content in the   previous reply",
]

HANDOFF_CASES: list[tuple[Any, Any]] = [
    ("what do you want", "Tell me your question or your task."),
    ("how can I help?", "I remain at your disposal."),
    ("something", "How may I assist you today?"),
    ("something", "How may I assist you?"),
    ("something", "How may I assist you now, friend?"),
    ("hello", "Tell me your question or your task."),
    ("Hi!", "Pray tell."),
    ("hey", "You're free to ask whatever you please."),
    ("good morning", "Without interruption, I am here."),
    ("check the weather", "Sunny today."),
    ("hi", ""),
    ("hi", None),
    (None, "Tell me your question or your task."),
    ("good evening!!!", "I'm ready to help with automation, spreadsheets, or any factual questions you have"),
]

ENVELOPE_TEXTS: list[Any] = [
    'Let me look. {tool_calls}{"tool": "search_web", "args": {}}{/tool_calls}',
    "Let me look. {tool_calls}",
    "Hi. { TOOL_CALLS }body{ / tool_calls }",
    "{tool_calls}a{/tool_calls} middle {tool_calls}b{/tool_calls}",
    "{somber}I'm sorry{/somber}",
    "{whisper}don't tell anyone{/whisper}",
    "{cheerful}that's wonderful{/cheerful}",
    "No markup here at all.",
    None,
    "",
    '{tool_calls}{"tool": "search_web", "args": {"query": "x"}}{/tool_calls}',
    '{tool_calls}[{"tool": "a", "args": {}}, {"tool": "b", "args": {"n": 1}}]{/tool_calls}',
    '{tool_calls}{"tool": "a", "args": {}}{/tool_calls}{tool_calls}{"tool": "b", "args": {}}{/tool_calls}',
    '{tool_calls}{"tool": "search_web", "args": "latest news"}{/tool_calls}',
    '{tool_calls}["search_web", {"tool": "a", "args": {}}]{/tool_calls}',
    "{tool_calls}{\"args\": {}}{/tool_calls}",
    '{tool_calls}{"tool": ""}{/tool_calls}',
    '{tool_calls}{"tool": "   "}{/tool_calls}',
    '{tool_calls}{"tool": null}{/tool_calls}',
    "{tool_calls}{/tool_calls}",
    "{tool_calls}   {/tool_calls}",
    "{tool_calls}not json{/tool_calls}",
    '{tool_calls}{"tool": "unterminated{/tool_calls}',
    "{tool_calls}5{/tool_calls}",
    '{tool_calls}"search_web"{/tool_calls}',
    "{tool_calls}null{/tool_calls}",
    "{tool_calls}true{/tool_calls}",
    "Just a plain reply.",
    "  {tool_calls}  {\"tool\": \"x\"}  {/tool_calls}  ",
]

# (id sequence, deque maxlen) for the dedupe struct.
EVENT_SEQUENCES: list[dict[str, Any]] = [
    {"ids": ["evt-1", "evt-1"], "maxlen": 5},
    {"ids": ["evt-1", "evt-2"], "maxlen": 5},
    {"ids": ["a", "b", "a"], "maxlen": 1},
    {"ids": ["a", "b", "c", "a"], "maxlen": 3},
    {"ids": [None, None], "maxlen": 5},
    {"ids": [], "maxlen": 5},
]


def capture_quality(reply: Any) -> None:
    booleans = {
        "looks_like_action_teaser_reply": [
            {"input": value, "expected": reply.looks_like_action_teaser_reply(value)}
            for value in TEASER_TEXTS
        ]
        + [
            {
                "input": teaser,
                "expected": reply.looks_like_action_teaser_reply(teaser),
            }
            for teaser in (
                # The word cap, from each side of it.
                "Let me check. " + " ".join(["ok"] * (reply.TEASER_MAX_WORDS - 3)),
                "Let me check. " + " ".join(["ok"] * (reply.TEASER_MAX_WORDS - 2)),
            )
        ]
        + [{"input": TEASER_LONG, "expected": reply.looks_like_action_teaser_reply(TEASER_LONG)}],
        "looks_like_mctrader_action_request": [
            {"input": value, "expected": reply.looks_like_mctrader_action_request(value)}
            for value in TRADER_TEXTS
        ],
        "looks_incomplete": [
            {
                "reply_text": text,
                "response_details": details,
                "expected": reply.looks_incomplete(text, details),
            }
            for text, details in INCOMPLETE_CASES
        ],
        "latest_user_message_text": [
            {"messages": value, "expected": reply.latest_user_message_text(value)}
            for value in LATEST_USER_CASES
        ],
        "looks_like_task_stub_reply": [
            {
                "user_text": user,
                "reply_text": text,
                "expected": reply.looks_like_task_stub_reply(user, text),
            }
            for user, text in TASK_STUB_CASES
        ],
        "merge_completed_reply": [
            {
                "partial": partial,
                "completion": completion,
                "expected": reply.merge_completed_reply(partial, completion),
            }
            for partial, completion in MERGE_CASES
        ],
        "extract_reply_segments": [
            {"value": value, "expected": reply.extract_reply_segments(value)}
            for value in SEGMENT_VALUES
        ],
        "strip_tool_call_envelope": [
            {"input": value, "expected": reply.strip_tool_call_envelope(value)}
            for value in ENVELOPE_TEXTS
        ],
        "tool_calls_in_envelope": [
            {"input": value, "expected": reply.tool_calls_in_envelope(value)}
            for value in ENVELOPE_TEXTS
        ],
        "looks_unusable_reply": [
            {"input": value, "expected": reply.looks_unusable_reply(value)}
            for value in UNUSABLE_TEXTS
        ],
        "looks_out_of_character_reply": [
            {"input": value, "expected": reply.looks_out_of_character_reply(value)}
            for value in OUT_OF_CHARACTER_TEXTS
        ],
        "looks_like_recovery_meta_reply": [
            {"input": value, "expected": reply.looks_like_recovery_meta_reply(value)}
            for value in RECOVERY_META_TEXTS
        ],
        "looks_like_generic_handoff_reply": [
            {
                "user_text": user,
                "reply_text": text,
                "expected": reply.looks_like_generic_handoff_reply(user, text),
            }
            for user, text in HANDOFF_CASES
        ],
    }
    events = []
    for case in EVENT_SEQUENCES:
        # One lock and one bounded deque for the whole sequence, as the app holds them.
        lock = threading.Lock()
        deque: collections.deque = collections.deque(maxlen=case["maxlen"])
        expected = [reply._is_duplicate_event(item, lock, deque) for item in case["ids"]]
        events.append(
            {
                "maxlen": case["maxlen"],
                "ids": case["ids"],
                "expected": expected,
                "final": list(deque),
            }
        )
    write_fixture(
        "quality.json",
        {
            "constants": {
                "REPLY_SEGMENT_MAX": reply.REPLY_SEGMENT_MAX,
                "TEASER_MAX_WORDS": reply.TEASER_MAX_WORDS,
            },
            "events": events,
            **booleans,
        },
    )


# --------------------------------------------------------------------------
# document
# --------------------------------------------------------------------------
DOCUMENT_CONTENTS: list[Any] = [
    "",
    None,
    "   ",
    '{"reply": "Plain JSON reply.", "tool_calls": []}',
    '{"reply": "One beat.", "replies": ["First beat.", "Second beat."]}',
    '{"reply": "Explicit wins.", "replies": ["Only one."]}',
    '{"response": "The alias key."}',
    "{'reply': 'A Python dict repr', 'tool_calls': [],}",
    "{'reply': 'None valued', 'tool_calls': None}",
    '{"reply": "A double encoded document."}',
    json.dumps(json.dumps({"reply": "Double encoded, really.", "tool_calls": []})),
    '```json\n{"reply": "Fenced.", "tool_calls": []}\n```',
    '```\n{"reply": "Fenced without a tag.", "tool_calls": []}\n```',
    'Sure — here you go: {"reply": "After a preamble.", "tool_calls": []}',
    'Prose then a broken doc: {"reply": "no closing brace"',
    '{"reply": "truncated reply, no quote at all',
    '{"reply": "truncated then tool_calls", "tool_calls": ["x"',
    'Not JSON at all, just a reply.',
    'Let me look. {tool_calls}{"tool": "search_web", "args": {"query": "x"}}{/tool_calls}',
    '{"reply": "Now {tool_calls}\\"tool\\": \\"lookup\\" more", "tool_calls": []}',
    '{"reply": "A lone {tool_calls} tag stays out.", "tool_calls": []}',
    '{"reply": "One\\nTwo\\n\\nThree", "tool_calls": []}',
    '{"replies": ["Only one segment."]}',
    '{"reply": 42}',
    '{"reply": null, "tool_calls": []}',
    '{"tool_calls": [{"tool": "get_positions", "args": {"brokerId": 1}}], "reply": "Tool first."}',
    json.dumps(
        {
            "reply": "Fetching positions...",
            "tool_calls": [{"tool": "get_positions", "args": {"brokerId": 1}}],
        }
    ),
    '{"reply": "Mr. Smith said it.Did he agree?", "tool_calls": []}',
    '{"extra_key": "kept", "reply": "With an extra key.", "tool_calls": []}',
    '{"reply": "Segments {tool_calls}\\"tool\\": \\"a\\" {/tool_calls} here.", "replies": ["First {tool_calls}\\"tool\\": \\"a\\" {/tool_calls}", "Second clean."]}',
    '{"replies": ["{tool_calls}\\"tool\\": \\"a\\" {/tool_calls}", "Second clean."]}',
    "[1, 2, 3]",
    '"a bare string"',
    "5",
]

CANDIDATES: list[Any] = [
    '{"reply": "json"}',
    "{'reply': 'literal'}",
    "{'reply': 'trailing', 'tool_calls': [],}",
    "{'reply': None, 'tool_calls': None}",
    "{'other': 'no reply key'}",
    "{not a literal at all}",
    "[1, 2]",
    "5",
    "{'nested': {'a': [1, (2, 3), True, None]}}",
    '{"reply": "json", "tool_calls": [{"tool": "x", "args": {}}]}',
    "",
    "'just a string'",
]

PLAINS: list[Any] = [
    "Nothing to strip here.",
    'Let me look. {tool_calls}{"tool": "search_web", "args": {}}{/tool_calls}',
    "Let me look. {tool_calls}",
    "Mr. Smith said it.Did he agree?",
    "",
    "{tool_calls}{}{/tool_calls}",
]


def capture_document(agent: dict[str, Any]) -> None:
    app = agent["app"]

    def parse_document(content: Any) -> dict[str, Any]:
        app.logger.warnings.clear()
        parsed = agent["_parse_model_reply_document"](content)
        return {
            "input": content,
            "expected": parsed,
            "warnings": [[message, list(args)] for message, args in app.logger.warnings],
        }

    def settle(parsed: Any) -> dict[str, Any]:
        app.logger.warnings.clear()
        settled = agent["settle_prose_tool_calls"](json.loads(json.dumps(parsed)))
        return {
            "input": parsed,
            "expected": settled,
            "warnings": [[message, list(args)] for message, args in app.logger.warnings],
        }

    def authority(content: Any) -> dict[str, Any]:
        app.logger.warnings.clear()
        settled = agent["parse_model_json_content"](content)
        return {
            "input": content,
            "expected": settled,
            "warnings": [[message, list(args)] for message, args in app.logger.warnings],
        }

    documents = [parse_document(content) for content in DOCUMENT_CONTENTS]
    authorities = [authority(content) for content in DOCUMENT_CONTENTS]
    settles = [settle(document["expected"]) for document in documents]

    write_fixture(
        "document.json",
        {
            "reply_document_keys": list(agent["REPLY_DOCUMENT_KEYS"]),
            "parse_model_reply_document": documents,
            "parse_model_json_content": authorities,
            "settle_prose_tool_calls": settles,
            "parse_structured_candidate": [
                {"input": value, "expected": agent["_parse_structured_candidate"](value)}
                for value in CANDIDATES
            ],
            "reply_without_prose_tool_calls": [
                {"input": value, "expected": agent["reply_without_prose_tool_calls"](value)}
                for value in PLAINS
            ],
        },
    )


# --------------------------------------------------------------------------
# scanner
# --------------------------------------------------------------------------
# The scanner's own limit is not available at module scope, so the overlong-preamble case is a
# marker that `capture_scanner` expands with the real constant.
OVERLONG_PREAMBLE = "@@OVERLONG_PREAMBLE@@"

SCAN_DOCUMENTS: list[Any] = [
    '{"reply": "Hello there.", "tool_calls": []}',
    '{"reply": "One two three.", "tool_calls": []}',
    '{"response": "Still fine."}',
    '{  "reply"\n\t:\n  "Spaced out."  }',
    '```json\n{"reply": "Fenced."}\n```',
    r'{"reply": "Line\nbreak \"quoted\" tab\tend."}',
    '{"reply": "caf\\u00e9 done."}',
    '{"reply": "wave \\uD83D\\uDE00 end."}',
    '{"reply": "broken \\uD83D end."}',
    '{"reply": "unfinished \\uD83D',
    '{"reply": "Stop here.", "tool_calls": [{"tool": "x"}]}',
    '{"tool_calls": [{"tool": "search_web"}], "reply": "Late."}',
    'Sure, here is my answer: {"reply": "Ignored."}',
    "The answer is forty-two.",
    '{"reply": 42}',
    OVERLONG_PREAMBLE,
    "nonsense",
    r'{"reply": "a \q b."}',
    '{"reply": "First \n second."}',
    '{"reply": ""}',
    '{"reply": "a \\ubad\\u0041 b."}',
    '{"reply": "tail \\uD83D\\u0041 end."}',
    '{"reply": "\\udc00 lone low."}',
    '{"Reply": "wrong case."}',
    '{"rep": "prefix that is not a key."}',
    "  \n\t {\"reply\": \"lead whitespace.\"}",
    "```json {\"reply\": \"fence with a space.\"}",
    '{"reply": "ends with a backslash \\\\"}',
]


def scan_with(cls: Any, chunks: list[Any]) -> dict[str, Any]:
    scanner = cls()
    collected = []
    for chunk in chunks:
        emitted = scanner.feed(chunk)
        if emitted:
            collected.append(emitted)
    collected.append(scanner.finish())
    return {
        "chunks": chunks,
        "text": "".join(collected),
        "reply": scanner.reply,
        "done": scanner.done,
        "aborted": scanner.aborted,
    }


def capture_scanner(agent: dict[str, Any]) -> None:
    cls = agent["JsonReplyStreamScanner"]
    overlong = "x" * (cls.MAX_PREAMBLE_CHARS + 10)
    cases = []
    for document in SCAN_DOCUMENTS:
        document = overlong if document == OVERLONG_PREAMBLE else document
        sequences = chunk_sequences(document)
        whole = scan_with(cls, sequences[0]["chunks"])
        for sequence in sequences:
            sequence.update(scan_with(cls, sequence["chunks"]))
            sequence["same_as_whole"] = sequence["text"] == whole["text"]
        cases.append({"document": document, "sequences": sequences})

    # The two behaviours that are not a document's chunk sequence.
    scanner = cls()
    ignored = {
        "feed_empty": scanner.feed(""),
        "feed_none": scanner.feed(None),
        "reply": scanner.reply,
        "aborted": scanner.aborted,
    }
    terminal = scan_with(cls, ["nonsense"])
    after = cls()
    after.feed("nonsense")
    terminal["later_feed"] = after.feed('{"reply": "Too late."}')
    terminal["later_reply"] = after.reply
    write_fixture(
        "stream_scanner.json",
        {
            "max_preamble_chars": cls.MAX_PREAMBLE_CHARS,
            "keys": list(cls.KEYS),
            "cases": cases,
            "empty_and_none_chunks": ignored,
            "abort_is_terminal": terminal,
        },
    )


# --------------------------------------------------------------------------
# splitter
# --------------------------------------------------------------------------
SPLITTER_CASES: list[dict[str, Any]] = [
    {"label": "terminators", "text": "Let me check that for you. It is forty-two! Are you sure?"},
    {"label": "chunking", "text": "First sentence here. Second sentence there. Third one to finish."},
    {"label": "abbreviations", "text": "I spoke to Mr. Smith about it. He agreed with me."},
    {"label": "dotted_acronyms", "text": "The U.S. market is up today. That is good news."},
    {"label": "decimals", "text": "The value is 3.5 percent higher. That matters."},
    {"label": "version_numbers", "text": "We are on version 2. It works well enough."},
    {"label": "ellipsis", "text": "Well now then... that is a long story indeed. Sit down."},
    {"label": "short_fragments", "text": "Ha. Yes. That is very funny indeed."},
    {"label": "tone_tag", "text": "She said {somber}I am sorry{/somber} to hear it. Then she left."},
    {
        "label": "scene_bracket",
        "text": "He paused for a moment. [the kettle whistles. nobody moves] Then he spoke again.",
    },
    {
        "label": "aside",
        "text": "It rained all week. (which is normal here. everyone knows it) We stayed in.",
    },
    {
        "label": "unclosed_opener",
        "text": "It rained all week. (which is normal here. everyone knows it",
    },
    {"label": "emphasis", "text": "He was *very. truly. annoyed* about it. We noticed."},
    {"label": "trailing_text", "text": "A complete sentence here. And then some text"},
    {"label": "dangling_terminator", "text": "Just one sentence now."},
    {"label": "whitespace_only", "text": "   \n  "},
    {
        "label": "not_on_a_comma",
        "text": "This sentence runs on and on, " + ("and on " * 60) + "and finally stops",
    },
    {
        "label": "on_a_semicolon",
        "text": "First clause here; " + ("and on " * 60) + "and finally stops",
    },
    {"label": "unpunctuated", "text": "word " * 200},
    {"label": "stripped", "text": "  One thing here.   Two things there!  Three.  "},
    {
        "label": "all_the_words",
        "text": "Right, let me check that for you. The U.S. market is up 3.5 percent, "
        "which is good news... Anyway, that is the lot.",
    },
    {"label": "underscores", "text": "He went _quiet. truly quiet_ about it. We noticed."},
    {"label": "mixed_markup", "text": "{somber}Sorry.[aside] A second beat here. Yes it is."},
]

SPLITTER_CHUNK_SIZES = [1, 2, 3, 5, 7, 30, 40, 50, 100, 1000]


def split_with(cls: Any, chunks: list[Any]) -> list[str]:
    splitter = cls()
    sentences: list[str] = []
    for chunk in chunks:
        sentences.extend(splitter.feed(chunk))
    sentences.extend(splitter.flush())
    return sentences


def capture_splitter(agent: dict[str, Any]) -> None:
    cls = agent["SentenceSplitter"]
    cases = []
    for case in SPLITTER_CASES:
        text = case["text"]
        whole = split_with(cls, [text])
        per_size = {}
        for size in SPLITTER_CHUNK_SIZES:
            chunks = [text[index : index + size] for index in range(0, len(text), size)]
            per_size[str(size)] = split_with(cls, chunks)
        seeded = {}
        for seed, pieces in ((20261120, 4), (47424, 9)):
            seeded[str(seed)] = split_with(cls, seeded_chunks(text, seed, pieces))
        # A feed that ends mid-markup holds the boundary; flush still releases the words.
        splitter = cls()
        fed = []
        for chunk in [text[index : index + 3] for index in range(0, len(text), 3)]:
            got = splitter.feed(chunk)
            if got:
                fed.append(got)
        first_flush = splitter.flush()
        cases.append(
            {
                "label": case["label"],
                "text": text,
                "whole": whole,
                "by_chunk_size": per_size,
                "seeded": seeded,
                "feed_groups": fed,
                "flush": first_flush,
                "flush_again": splitter.flush(),
            }
        )
    write_fixture(
        "splitter.json",
        {
            "min_sentence_chars": cls.MIN_SENTENCE_CHARS,
            "max_sentence_chars": cls.MAX_SENTENCE_CHARS,
            "common_non_terminal_abbreviations": sorted(
                agent["COMMON_NON_TERMINAL_ABBREVIATIONS"]
            ),
            "cases": cases,
        },
    )


# --------------------------------------------------------------------------
# publisher + the streamed-prefix rules
# --------------------------------------------------------------------------
PUBLISH_REPLIES: list[Any] = [
    "Let me check that for you. It is forty-two. That is all.",
    "First sentence arrives now. Second one is later on.",
    "One two three four. Five six seven eight.",
    "A sentence here. Another sentence there.",
    "Spoken to nobody. But counted.",
    "Well now then... that is a long story indeed. Sit down.",
    "She said {somber}I am sorry{/somber} to hear it. Then she left.",
    "No terminators at all in this one",
    "",
    "  ",
]

PREFIX_CASES: list[dict[str, Any]] = [
    {
        "reply": "Let me check that for you. It is forty-two.",
        "spoken": "Let me check that for you.",
    },
    {"reply": "That is the whole thing.", "spoken": "That is the whole thing."},
    {"reply": "Whole reply here.", "spoken": ""},
    {
        "reply": "One two three.   Four five six.\nSeven eight nine.",
        "spoken": "One two three. Four five six.",
    },
    {
        "reply": "Something entirely different was said instead.",
        "spoken": "The beginning the user actually heard.",
    },
    {"reply": "Anything at all.", "spoken": ""},
    {
        "reply": "Let me check that now. The market is up three percent.",
        "spoken": "Let me check that now.",
    },
    {"reply": "The market is up three percent.", "spoken": "Let me check that now."},
    {"reply": "Unchanged reply.", "spoken": ""},
    {"reply": "Let me check that now.", "spoken": "Let me check that now."},
    {"reply": "", "spoken": ""},
    {"reply": None, "spoken": None},
    {"reply": "  spaced reply  ", "spoken": "spaced reply"},
    {"reply": "abc", "spoken": "abcd"},
    {"reply": "Partial match of the wor", "spoken": "Partial match of the word"},
]


def publish_with(cls: Any, chunks: list[Any]) -> dict[str, Any]:
    published: list[str] = []
    publisher = cls(published.append)
    for chunk in chunks:
        publisher.feed(chunk)
    spoken = publisher.finish()
    return {
        "chunks": chunks,
        "published": published,
        "spoken": spoken,
        "aborted": publisher.aborted,
        "count": publisher.published,
        "spoken_text": publisher.spoken_text(),
    }


def capture_publisher(agent: dict[str, Any]) -> None:
    cls = agent["ReplyStreamPublisher"]
    cases = []
    for reply in PUBLISH_REPLIES:
        document = json.dumps({"reply": reply, "tool_calls": []})
        sequences = []
        for size in (5, 1, 40, len(document) or 1):
            chunks = [document[index : index + size] for index in range(0, len(document) or 1, size)]
            sequences.append({"label": f"size-{size}", **publish_with(cls, chunks)})
        for seed, pieces in ((20261120, 3), (47424, 7)):
            sequences.append(
                {"label": f"seeded-{seed}", **publish_with(cls, seeded_chunks(document, seed, pieces))}
            )
        cases.append({"reply": reply, "document": document, "sequences": sequences})

    # The shapes the ported tests build by hand rather than from a whole reply: mostly the abort
    # paths, which a document built from `PUBLISH_REPLIES` can never reach. Whatever Python does
    # with these is what the Rust port is pinned to — including how much was published before an
    # abort, which is the part a reader cannot guess.
    half = json.dumps(
        {"reply": "First sentence arrives now. Second one is later on.", "tool_calls": []}
    )
    hand_built = [
        {
            "label": "tool_calls_first",
            "why": "the scanner aborts before any reply text exists",
            **publish_with(cls, ['{"tool_calls": [], "reply": "Too late."}']),
        },
        {
            "label": "prose_then_document",
            "why": "prose is published, then a document reveals tool calls",
            **publish_with(cls, ["Let me look. ", '{"tool_calls": [], "reply": "Later."}']),
        },
        {
            "label": "document_opens_then_tool_calls",
            "why": "the abort lands after the document has opened",
            **publish_with(cls, ['{"tool_calls": []', ', "reply": "Never spoken."}']),
        },
        {
            "label": "half_a_document",
            "why": "finish() on a document that never closed",
            **publish_with(cls, [half[: len(half) // 2]]),
        },
        {
            "label": "empty_chunks",
            "why": "a stream of nothing, which must not invent a sentence",
            **publish_with(cls, ["", "", ""]),
        },
    ]

    streamed = []
    for case in PREFIX_CASES:
        prefix_length = agent["_streamed_prefix_length"](
            case["reply"] or "", case["spoken"] or ""
        )
        remaining, replaced = agent["split_reply_after_stream"](case["reply"], case["spoken"])
        streamed.append(
            {
                **case,
                "prefix_length": prefix_length,
                "split": [remaining, replaced],
                "stripped": agent["strip_streamed_prefix"](case["reply"], case["spoken"]),
            }
        )
    write_fixture(
        "publisher.json",
        {
            "cases": cases,
            "hand_built": hand_built,
            "streamed": streamed,
        },
    )


def main() -> None:
    import utils.reply as reply  # imports json/re/typing only

    agent = load_agent_range(agent_namespace())
    print(f"captured from {REFERENCE_APP} (agent.py:{RANGE_START}-{RANGE_END})")
    capture_reply_text(agent)
    capture_quality(reply)
    capture_document(agent)
    capture_scanner(agent)
    capture_splitter(agent)
    capture_publisher(agent)


if __name__ == "__main__":
    main()
