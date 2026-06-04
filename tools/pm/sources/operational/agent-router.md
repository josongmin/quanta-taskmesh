## Read First — Non-Negotiables

This repo is the **Eleven Labs logq take-home**: a small log search tool that
must be **faster than grep in practice** but **not** a thin wrapper around
`grep`/`rg`. Queries must be backed by an **index or equivalent structure** you
build and own.

Assignment hard rules (see `docs/00.requirements.md`):

- **Scope:** ~6 hours of focused work. Prefer fewer features with depth over a
  shallow checklist.
- **Deliverable:** runnable `logq` (CLI or HTTP — pick one and document it).
- **Forbidden:** Lucene, Tantivy, Bleve, Elasticsearch clients, or any library
  that *is* a search/index engine. JSON, regex, datetime, compression, etc. are
  fine.
- **Data:** three mixed formats (JSON Lines, Apache combined, key=value) plus
  ~1% broken lines from `tools/generate_logs.py`. **Inspect `sample_logs/`**
  before locking index/query design; justify choices in `DESIGN.md` as
  “I observed X in the dataset, therefore Y.”
- **Docs:** ship `README.md`, `DESIGN.md`, `MEASUREMENTS.md`, `AI_USAGE.md`
  with real measured numbers only in `MEASUREMENTS.md`.
- **Agent guidance:** edit only `tools/pm/sources/**`, then run
  `python3 tools/pm/pm.py sync`. Never hand-edit `AGENTS.md`, `CLAUDE.md`,
  `.codex/*`, or `.cursorrules`.

Engineering non-negotiables for this codebase:

- **No query-time full-file scan** as the primary search path once ingest exists.
  Answer from your index/store.
- **No shelling out** to `grep`/`rg`/`awk` for search execution.
- **Parse failures are explicit:** broken lines are counted/skipped/stored as
  raw per your documented policy — not silently dropped or “fixed” with guesses.
- **Deterministic output:** stable sort order for persisted index metadata and
  for rendered query results (tests and `MEASUREMENTS.md` depend on this).

## Where Things Live

| Need | Location |
| --- | --- |
| Assignment spec | `docs/00.requirements.md` |
| Synthetic data generator | `tools/generate_logs.py` |
| Generated sample data (gitignored) | `sample_logs/` (`app.log`, `access.log`, `metrics.log`) |
| Agent/operator instruction sources | `tools/pm/sources/**` |
| Prompt manager CLI | `tools/pm/pm.py`, `tools/pm/targets.yaml` |
| Python tooling config | `pyproject.toml` |
| User-facing runbook (you write) | `README.md` |
| Design rationale (you write) | `DESIGN.md` |
| Measured perf/memory (you write) | `MEASUREMENTS.md` |
| AI disclosure (you write) | `AI_USAGE.md` |
| `logq` Rust workspace | `crates/` (see `tools/pm/sources/rules/architecture.md`) |

Generated docs (`AGENTS.md`, `CLAUDE.md`, `.codex/*`, `.cursorrules`) are
produced from `tools/pm/sources/**`. Never hand-edit them.
