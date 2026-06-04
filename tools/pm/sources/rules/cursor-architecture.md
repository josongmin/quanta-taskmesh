## Cursor — Architecture Enforcement (read before every edit)

This repo is **not** a flat Rust project. Cursor must not paste code from other
repos (`/Users/songmin/logq`, Elasticsearch examples, generic CLI templates)
without re-homing it to the **allowed crate graph** below. Violations fail
`just test-architecture`, semgrep, and `just gate`.

### Crate graph (only these workspace edges)

```text
logq-value        -> (none)
logq-clock        -> logq-value
logq-segment      -> logq-value          # concrete store/codec/segment only — NO traits
logq-ingest       -> logq-value, logq-clock, logq-segment
logq-search       -> logq-value, logq-clock, logq-segment
logq-cli          -> logq-value, logq-clock, logq-ingest, logq-search
logq-test-support -> logq-value, logq-clock, logq-segment
logq-tests        -> (dev-deps on workspace crates)
```

**Hard bans:**

| Action | Why |
| --- | --- |
| `logq-ingest` imports `logq-search` (or reverse) | Features compose only at `logq-cli` |
| `logq-segment` defines port traits | ADR-0000: traits live in feature `ports/` |
| `domain/` or `ports/` imports `adapters/` | Hexagonal direction is inward-only |
| `std::fs` / `File::open` in `logq-value`, `logq-clock`, or feature `domain/` | IO belongs in adapters or `logq-segment` |
| `println!` / `process::exit` outside `logq-cli` | CLI is the only IO surface |
| `clap` outside `logq-cli` | Parse argv once at the edge |
| New dependency on a search-engine crate | Assignment + `cargo deny` |
| `grep`/`rg` subprocess for search | Not an indexed engine |

### Where new code goes

| You are implementing… | Put it in… | Do NOT put it in… |
| --- | --- | --- |
| Segment file format, manifest, `pread`, postings codec | `crates/logq-segment/src/{store,segment,codec}.rs` | `logq-ingest`, `logq-search` |
| Parser recognizers (JSONL / Apache / kv) | `logq-ingest/src/adapters/parsers/` | `logq-segment` |
| Ingest orchestration, rollover, source state | `logq-ingest/src/domain/` + `ports/` + `adapters/` | `logq-search` |
| Query AST, planner, DAAT merge | `logq-search/src/domain/` + `ports/` + `adapters/` | `logq-ingest` |
| `logq ingest` / `logq search` wiring | `logq-cli/src/` | inside segment or domain |
| Cross-feature ingest→search proof | `crates/logq-tests/tests/` | random crate `tests/` |

### `logq-segment` rules (common Cursor mistakes)

- **Concrete types only** — `SegmentStore`, `SegmentView`, `SegmentBuild`, codec
  helpers. No `trait SegmentStore` in this crate.
- **Public API** is what `lib.rs` re-exports. Internal helpers stay `pub(crate)`;
  do not add `pub fn` on `codec::Reader` unless it is part of the crate's public `lib.rs` API.
- **Errors** — use `SegmentError`; never `map_err(|_| …)` (clippy
  `map_err_ignore`). Preserve the source error in the message.
- **No silent defaults on corruption** — decode failures return
  `SegmentError::Segment` / `Corruption`, not empty `Vec` or `Ok(None)` unless
  that is the documented domain answer (e.g. dictionary miss).
- **Tests** for segment behavior go in `crates/logq-segment/tests/` or
  `#[cfg(test)]` in the same module — not in `logq-cli`.

### Before submitting a Cursor diff

1. List every file you touched and which crate/layer owns it.
2. Run `just test-architecture` if any `Cargo.toml` or `use` path changed.
3. Run `just clippy` (two passes — includes pedantic/nursery + production restrict).
4. If you edited `tools/pm/sources/**`, run `just pm-sync` and `just pm-lint`.

If the change needs a new crate edge, **stop** and update `docs/adr/` + the
architecture checker — do not “just add the import” to make it compile.
