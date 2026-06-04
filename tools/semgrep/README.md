# taskmesh Semgrep rule pack

Machine-enforced architecture and production policy for the workspace.

Target crate graph (ADR-0001):

- `taskmesh-contract` — vocabulary + port traits (`Runtime`, `Executor`, `Clock`, `MemoryProbe`)
- `taskmesh-engine` — pure governance core (no tokio/rayon)
- `taskmesh` — host facade + tokio driven adapters
- `taskmesh-rayon` — `Executor` adapter only (no tokio)

During the `taskmesh-core` → `taskmesh-engine` rename, rules also scan `crates/taskmesh-core/**`
and `crates/taskmesh-tokio/**` where noted.

```bash
semgrep --config tools/semgrep/rules --error crates
# or: just semgrep
```

Rule ids use the `taskmesh-` prefix. Opt out with `// nosemgrep: <id> -- reason: …`
(same-line reason required; see `error-handling.yml`).
