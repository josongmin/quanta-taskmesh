## Working Rules

Keep a **thin boundary** between parsing, indexing, and the user interface across
the Rust workspace:

| Layer | Location | Responsibility |
| --- | --- | --- |
| **Kernel** | `logq-value`, `logq-clock` | Shared types and clock port; no I/O |
| **Segment infra** | `logq-segment` | On-disk index structures (concrete types) |
| **Feature domain** | `logq-ingest`, `logq-search` `src/domain/` | Use-case logic; no adapters |
| **Feature ports** | `src/ports/` | Trait definitions |
| **Feature adapters** | `src/adapters/` | Parser, fs, segment I/O implementations |
| **CLI** | `logq-cli` | argv, stdout/stderr, exit codes, composition |

On any change:

- **Parsers or field canonicalization** → add parser tests with real snippets
  from `sample_logs/` (including broken lines).
- **Index format or ingest** → add round-trip or ingest→search integration tests.
- **Query syntax** → document examples in `README.md` and lock behavior with tests.
- **Agent guidance** → edit `tools/pm/sources/**`, run `python3 tools/pm/pm.py sync`.
- **Performance claims** → re-run measurements and update `MEASUREMENTS.md` with
  the exact command and dataset size used.

Prioritize **at least four Core requirements** with clear “not implemented”
notes for the rest in `README.md` (the assignment treats that section as
important signal).
