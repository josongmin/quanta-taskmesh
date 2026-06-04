## Determinism

Search output and persisted index metadata should be **reproducible** for the
same inputs and query.

Rules:

- Sort doc ids / timestamps / tie-breakers before returning results when the
  assignment does not require relevance ranking.
- When serializing the index to disk, use stable key ordering (e.g. sorted JSON
  keys, or explicit schema version + sorted runs).
- Do not depend on `dict` iteration order for anything written to disk or printed
  in golden tests.
- For time filters (`--since`, `--from`/`--to`), document whether timestamps are
  UTC, which field is authoritative per format, and how malformed timestamps are
  rejected.
- Randomness is only for **data generation** (`tools/generate_logs.py`), not for
  query results or doc id assignment unless seeded and documented.

Why: stable output makes pytest snapshots trustworthy and makes
`MEASUREMENTS.md` comparable across runs on the same machine.

### No silent heuristics

- **Format detection:** each format gets an explicit recognizer; avoid routing
  solely on the first character.
- **Field types:** document coercion (`user_id` int vs str) in `DESIGN.md` with
  a test per rule.
- **Truncation:** if you enforce a memory budget, surface it (error or metric) —
  do not silently stop indexing mid-file without recording that fact.
