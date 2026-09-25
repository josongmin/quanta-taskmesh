# taskmesh Semgrep rule pack

Machine-enforced architecture and production policy for the workspace.

Target crate graph (ADR-0001):

- `taskmesh-contract` — vocabulary + port traits (`Runtime`, `CpuExecutor`, `PermitWaker`, `Clock`)
- `taskmesh-engine` — pure governance core (no tokio/rayon)
- `taskmesh` — host facade + tokio driven adapters
- `taskmesh-rayon` — `CpuExecutor` adapter only (no tokio in `src/`; its integration tests may drive it from tokio)

Scope of the boundary rules (`architecture.yml`, `dependency-policy.yml`): every file under a
crate's `src/`, including `*_tests.rs` and `src/tests/` modules — a forbidden import compiled into
the crate is a violation wherever it sits. Only `crates/<crate>/tests/` (integration tests, which
may use dev-dependencies) and benches are out of scope. `tools/arch/check_crate_boundaries.py`
independently verifies that in-`src` test modules are declared under `#[cfg(test)]`.

Scope of the test-quality rules (`test-quality.yml`): every `#[test] fn`, `#[tokio::test] async fn`,
and `#[tokio::test(...)] async fn` body under `crates/`. Each rule has a fire/clean fixture in
`tests/test_rules_fire.py`; a rule without one is not considered enforced.
The gate compares Semgrep's scanned paths with every present Rust file under
`crates/`, including production modules, unit-test modules, integration tests,
benchmarks, examples, and support modules. Omitting any one file fails even if
the rest of its crate was scanned.

```bash
uv run python tools/semgrep/check.py
# or: just semgrep
```

Rule ids use the `taskmesh-` prefix. Opt out with `// nosemgrep: <id> -- reason: …`
(same-line reason required; see `error-handling.yml`).
