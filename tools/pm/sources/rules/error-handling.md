## Error Handling

A search tool that **drops failures quietly** is worse than one that errors
loudly on ingest.

Principles:

- **Parse errors:** per-line failures should follow a single documented policy
  (skip + count, store raw blob, quarantine file). Never pretend a broken line
  parsed successfully.
- **Ingest errors:** missing files, permission errors, and corrupt index state
  should abort or return a non-zero exit with a clear message.
- **Query errors:** unknown field, bad operator, or impossible time range → typed
  error to the user, not an empty result that looks like “no matches”.
- **No silent recovery:** production code must not coerce I/O/decode failures
  into empty defaults (see semgrep error-handling rules).

Rust patterns:

| Avoid | Prefer |
| --- | --- |
| `.unwrap()` in production | `?` with a typed error enum |
| `.expect("")` / `.expect("TODO")` | `.expect("<invariant that cannot fail>")` |
| `Err(_) => {}` or `let _ = fallible();` | propagate with `?` or map to a typed error |
| `panic!()` / `todo!()` in shipping paths | return `Result` with an explicit variant |
| Logging only at DEBUG for skipped broken lines | count exposed in ingest summary |

CLI should map errors to exit codes consistently (document in `README.md`), e.g.:

- `0` — success (possibly zero hits)
- `1` — user error (bad query, bad flags)
- `2` — internal/unexpected failure
- `3` — missing/corrupt index (when applicable)

Be ready to explain one parse-failure example from `sample_logs/` and show
exactly what `logq` does with it.
