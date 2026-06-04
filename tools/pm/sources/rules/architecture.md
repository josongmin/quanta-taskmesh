## Architecture & Crate Boundaries

`logq` is a Rust log search engine built as a feature-axis workspace with
internal hexagonal layers. The crate graph is the architecture authority.

Allowed workspace edges:

```text
logq-value        -> (none)
logq-clock        -> logq-value
logq-segment      -> logq-value
logq-ingest       -> logq-value, logq-clock, logq-segment
logq-search       -> logq-value, logq-clock, logq-segment
logq-cli          -> logq-value, logq-clock, logq-ingest, logq-search
logq-test-support -> logq-value, logq-clock, logq-segment
logq-tests        -> (dev-deps on workspace crates)
```

Layout:

```text
crates/
  logq-value/          # leaf kernel: value, error, timestamp
  logq-clock/          # leaf kernel: Clock trait + impls
  logq-segment/        # shared infra: store, segment, codec
  logq-ingest/         # ingest feature: src/{domain,ports,adapters}/
  logq-search/         # search feature: src/{domain,ports,adapters}/
  logq-cli/            # composition root: clap + dispatch
  logq-tests/          # cross-feature integration tests
  logq-test-support/   # dev-only test helpers
```

Boundary rules:

- **Kernel** (`logq-value`, `logq-clock`): no filesystem, no environment reads,
  no wall-clock outside `logq-clock`, no terminal I/O.
- **Feature domain/ports** (`logq-ingest`, `logq-search`): traits and pure logic
  only — no `std::fs`, no concrete adapters, no cross-feature imports.
- **Feature adapters**: implement ports; may use `logq-segment` concrete types.
- **logq-cli**: argv parsing, stdout/stderr, exit codes; wires features together.

Forbidden usages (fail-closed):

- `std::fs` in kernel or feature domain/ports.
- `println!`, `eprintln!`, or `std::process::exit` outside `logq-cli`.
- `clap` outside `logq-cli`.
- Cross-feature imports (`logq-ingest` ↔ `logq-search`).

Enforcement: `tools/arch/check_crate_boundaries.py`, `tools/semgrep/rules/**`,
and `just clippy` (pedantic + production restriction lints). Violations fail
`just gate`.
