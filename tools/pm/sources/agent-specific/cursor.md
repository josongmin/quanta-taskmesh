## Cursor Notes (mandatory)

### Generated rules

- `.cursorrules` is **generated** from `tools/pm/sources/**`.
- **Never** hand-edit `.cursorrules`, `AGENTS.md`, or `.codex/*`.
- After changing sources: `just pm-sync` then `just pm-lint`.

### Architecture is not negotiable

Read **`cursor-architecture.md`** (included in this file) before every task.
Cursor frequently violates boundaries when porting code — treat that section as a
checklist, not background reading.

**Stop and re-plan** if your diff:

- Adds `pub` items in `logq-segment` that are not listed in `lib.rs` public exports
- Puts ingest logic in `logq-search` or search logic in `logq-ingest`
- Introduces traits into `logq-segment`
- Adds `std::fs` to `domain/` or `ports/`
- Copies a module from another repo without mapping it to the table in
  `cursor-architecture.md`

### Scoped edits

| Task | Allowed touch set |
| --- | --- |
| Segment store/codec | `crates/logq-segment/**` only |
| Ingest parser | `logq-ingest/src/adapters/parsers/**` + tests |
| Query engine | `logq-search/src/domain/**` + tests |
| CLI flags / exit codes | `logq-cli/**` only |
| Cross-feature behavior | `logq-tests` + minimal CLI wiring |

Do **not** “drive-by” refactor unrelated crates.

### Clippy / gate

- Production code (`--lib --bins`) uses **pedantic + nursery + restriction** lints.
- Fix violations; do not blanket-`allow` without a one-line reason on that item.
- `map_err(|_| …)` is **always wrong** — use `SegmentError::try_from_int` or
  include `{e}` in the message.
- Closeout for Rust changes: `just clippy` (must pass both passes).

### Data & docs

- Do not commit `sample_logs/` — `just generate-logs` or
  `just generate-logs 100MB` (positional size override).
- Record AI usage in `AI_USAGE.md` when you used Cursor for a design/code slice.
