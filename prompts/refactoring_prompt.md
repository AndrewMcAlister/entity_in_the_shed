# Refactoring Prompt — Scope: {SCOPE}

## Scope
Only the files listed below (and their direct test coverage in `tests/`) are in scope.
Do NOT modify files outside this scope except for:

- Updating import statements in call-site files
- Adding shared utilities to `utils/` if needed by multiple folders

## Step 1: Analyze
Review each `.py` file in scope. For each file:

1. Count lines. Flag any file >500 lines.
2. Identify methods/functions >50 lines.
3. Detect duplicated logic (same pattern in 2+ files).
4. Find methods with "and" in their name → split candidates.
5. Flag any `datetime.utcnow()` usage.
6. Flag any direct `self._private_method` access from tests.
7. List tests in `tests/` that cover the files in scope.
8. Summarize the current refactoring baseline (existing tools/checks).

## Step 2: Apply Refactoring Rules

### Rule A — Stateless Extraction
Any function/method that does NOT read `self`, `cls`, or mutate non-local state:
- Extract to a new or existing `utils/` module
- Update all call sites in scope
- Add unit tests in `tests/test_<module>_utils.py`

### Rule B — Atomic Methods
Split any method doing >1 distinct operation:
- Extract parameter builders (pure functions)
- Extract I/O wrappers (side-effect functions)
- Name methods with single verbs: `build_`, `validate_`, `persist_`, `fetch_`

### Rule C — Encapsulation & God Class Breakdown
For any class >500 lines in scope:
- Split into focused collaborator classes
- Keep original class as a thin facade if it has external call sites
- Move extracted classes to the target folder or its `modules/` subdirectory

### Rule D — Test Clean-Up
For every test that references a removed private method:
- Replace with public API call, OR
- Replace with direct utility function call, OR
- Delete if it tests implementation detail only

### Rule E — Warning Hygiene
Replace all `datetime.utcnow()` with `datetime.now(datetime.UTC)` in scope.

### Rule G — Prefer Concrete Types
Use concrete type annotations instead of `Any` or generic `dict`/`list` return types where the structure is known:
- Define `TypedDict`, `NamedTuple`, or dataclass return types for functions that return structured data.
- Use explicit type aliases (e.g. `JSON = dict[str, Any]`) only when the shape is genuinely dynamic.
- Annotate public function signatures with the most specific type practical — this improves IDE support and catches regressions earlier.

### Rule F — No Deadlock Introductions
Ensure any endpoint with I/O checks its backend or config BEFORE attempting any long-running operation. Specifically:
- Guard database backups (SQL Server / SQLite) with backend checks before `pyodbc.connect()`.
- Guard broker API calls with credential-availability checks before network I/O.

## Step 3: Verify Syntax
After all changes, run a syntax check on every modified file:

```bash
python -m py_compile <path>
```

Then run the scoped verification script if one exists (e.g. `scripts/verify_refactor.py`).

## Step 4: Run Scoped Tests
Execute ONLY tests that cover this folder:

```bash
python -m pytest tests/ -v --tb=short --timeout=30 \
  --deselect tests/test_backup_sqlite_database_returns_400_for_non_sqlite_backend \
  -k "<relevant markers or file patterns>"
```

All previously passing tests must remain passing. If any test fails:

1. Identify the root cause.
2. If it is a legitimate regression introduced by this refactor, fix it immediately.
3. If the test was already broken before this refactor, log it as a pre-existing issue.

## Step 5: Report
Produce a summary with:

### Changes Made
- Files modified
- Methods extracted/renamed/removed
- Tests updated
- New utility classes/modules created
- Dead code removed

### Test Results
- Passed / Failed / Skipped counts
- Any test failures with root cause
- Any pre-existing failures logged

### Blockers
- Any issues requiring human input
- Any scope-expansion recommendations (files outside this scope that should be refactored next)

### Refactoring Checklist

```
□ Methods extracted are stateless and dependency-free
□ Old method removed from original class (verified by verify_refactor.py where applicable)
□ All call sites updated (grep for old method name)
□ Tests updated to call new utility or public API
□ All previously passing tests still pass
□ New utility has its own unit tests
□ No new warnings introduced
□ No deadlocks or blocking I/O introduced