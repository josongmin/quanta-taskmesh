## Test Topology

Tests should **catch real regressions** in parsing, indexing, and search — not
merely import modules.

Placement:

| If it proves… | Put it in… |
| --- | --- |
| Kernel types / timestamp / value laws | `crates/logq-value/src/**` `#[cfg(test)] mod tests` |
| Clock trait behavior | `crates/logq-clock/src/**` unit tests |
| Segment codec / store round-trip | `crates/logq-segment/src/**` + `crates/logq-segment/tests/` |
| Parser recognizers | `crates/logq-ingest/src/adapters/parsers/**` unit tests |
| Query parse / exec algebra | `crates/logq-search/src/domain/**` unit tests |
| Ingest then search end-to-end | `crates/logq-tests/tests/integration/` |
| Recovery / concurrency rails | `crates/logq-tests/tests/{recovery,concurrency}/` |
| CLI argv, exit codes, stdout shape | `crates/logq-cli/tests/` or `tests/e2e/` |
| Crate boundary / forbidden usage | `just test-architecture` + semgrep |

Use **realistic fixtures**: copy short snippets from `sample_logs/` (valid JSONL,
Apache line, kv line, and at least one broken line). Do not only test toy strings.

Every shipped feature needs:

1. A focused unit test in the owning crate module, and
2. An integration test in `logq-tests` if behavior crosses domain → adapter → CLI.

Naming carries intent: `test_parse_*`, `test_ingest_*`, `test_search_*`,
`test_*_broken_line`, `test_*_regression`.

Avoid:

- Tests that only assert `True` or import without behavior checks.
- Tests that depend on the full 500MB dataset (keep fixtures small; use
  `generate_logs.py --lines` in docs for manual perf runs).

Run before closeout:

```bash
just test-ci
# or: cargo nextest run --workspace --profile ci
uv run pytest tools -q
just gate
```
