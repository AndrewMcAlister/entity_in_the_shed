# Preamble for an agent session

Read this once at the start of a session, before the slice brief. It is the house rules that are
the same every time; the brief is the task and `TODO/RESUME-NOTES.md` is where things stand.
Nothing here overrides `How_To_Code.md`.

## First actions

1. Read `D:\Entity_In_The_Shed\TODO\RESUME-NOTES.md` — "Where we are" says which slice is open and
   what the last session left behind.
2. Read your slice brief, `D:\Entity_In_The_Shed\TODO\SLICE-<name>-BRIEF.md` — it is the task, and
   its source is pasted inside it.
3. Start writing. Do not summarise the brief, do not plan out loud, do not search for the slice.

## Tools — the way this goes wrong

- Use the exact `name` from the `<tools>` block in your system prompt. In this install the tools come
  from MCP servers, so they carry the server prefix: `files__read_file`, `files__write_file`,
  `files__edit_file`, `files__grep_files`, `files__list_dir`, `files__glob_files`,
  `terminal__run_command`. Do not shorten them to the bare tool name and do not invent a spelling —
  what `/mcp` lists is what is declared.
- A name that is not declared is refused, the call goes back to the chat as text, and the turn ends
  with nothing done — which looks exactly like the model having stopped. The same happens when a turn
  is cut short (a small `max_tokens`, or the 24-round cap that used to exist): the last call is shown
  but never run.
- The call format is the one your template documents:

  ```
  <tool_call>
  <function=files__edit_file>
  <parameter=path>
  D:\Entity_In_The_Shed\...
  </parameter>
  <parameter=new_string>
  ...
  </parameter>
  </function>
  </tool_call>
  ```

- One tool call per reply, then wait for its result. If a call comes back as text instead of a
  result, retry it — with the declared name — rather than narrating what you are about to do.

## Paths and commands

- Every path handed to the files tool is absolute (`D:\Entity_In_The_Shed\...`). The files tool is
  rooted at `D:\` and the terminal starts in `D:\`, so a bare `TODO/...` resolves to `D:\TODO\...`
  and fails.
- Terminal commands start with `cd /d D:\Entity_In_The_Shed && `.
- Cargo runs as `cmd /c "cargo ... 2>&1"`: PowerShell turns cargo's stderr into noise and can leave
  a non-zero `$LASTEXITCODE` on a passing run.
- PowerShell separates commands with `;`, not `&&`. .NET APIs ignore the current directory — pass
  absolute paths to them too.

## Slice discipline

- One slice per turn, ending green and committed. Do not widen a slice, and do not merge two to
  "save a turn".
- The gate, run from `D:\Entity_In_The_Shed`:

  ```
  cargo fmt --all -- --check
  cmd /c "cargo clippy --workspace --all-targets -- -D warnings 2>&1"
  cmd /c "cargo test --workspace 2>&1"
  cmd /c "npm test 2>&1"
  ```

  Report the test total you get; 463 Rust tests were green at `20c420c` (Stage 4 part 2d).
- Then commit with the message the brief names, push, and add one line to `TODO/RESUME-NOTES.md`
  under "Where we are". Then stop.
- Gate red after two honest fix attempts, or the brief and the source disagree → stop and paste the
  output. Never adjust a green gate, never add an `#[allow]` to pass a lint the workspace denies.

## Reading

- Do not read `agent.py`. Every brief pastes the source it ports, verified against the reference.
- Do not read `utils/conversation_history.py` at all — it is 445 lines and the briefs carry the parts.
- The reference app is `D:\Entity-In-The-Shed` (hyphen); this clone is `D:\Entity_In_The_Shed`
  (underscore). Never boot the reference app or bind port 5005.
- The repo is the memory, not the conversation: a chat that hit its context is not resumed.
