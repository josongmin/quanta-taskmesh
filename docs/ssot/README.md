# Taskmesh operating kernel

This is a navigation and current-state index. It does not redefine API,
governance, gate membership, or release policy. Follow the owning source and
documents below when they change.

The current-source verdict and verification scope are in the
[2026-10-03 audit](../audits/2026-10-03-current-source-final.md).

## Source and evidence snapshot

- 2026-10-03 review baseline: clean Taskmesh `main@c4bcb2f213c66aac8adc7d3493c5a39917a29e00`, matching `origin/main` at inspection.
- GitHub `main` protection requires the strict `required 16-gate verdict` check. Its [main-push run](https://github.com/josongmin/quanta-taskmesh/actions/runs/36261855351) passed on that SHA. A later document or code commit needs a new exact-source result.
- `pr-ci.yml` is active on PR/main. Full `ci.yml`, `bench.yml`, and `release.yml` have manual-only source triggers and were disabled in GitHub settings at inspection. There is no scheduled full nightly run. `nightly` names a high-cost proof profile, not a cron schedule.
- The working tree now also contains Taskmesh host/API and measured-admission
  changes. Focused dirty-source Rust/Python and consumer-MSRV results are
  diagnostic. No new exact-source macOS CI, nightly, Linux release, performance,
  or Semantica consumer qualification was run. Release and external adoption
  remain OPEN.

## Authority map

| Question | Owner |
|---|---|
| Crate boundaries and public API | [ADR 0001](../adr/0001-hexagonal-feature-sliced-architecture.md), [library spec](../taskmesh-library-spec.md), `crates/taskmesh-contract/src` and `crates/taskmesh/src/lib.rs` |
| Task/stage/wire behavior | [external interface](../taskmesh-external-interface.md), [ADR 0004](../adr/0004-sep-25-ingress-plan-identity-and-wire.md) |
| Executor, deadline, response and capacity custody | [ADR 0005](../adr/0005-sep-25-execution-response-and-custody.md), host and engine source |
| Gate membership and commands | `tools/gates/required.json`, `tools/gates/inventory.json`, `Justfile`; [ADR 0006](../adr/0006-source-bound-verification-authority.md) defines evidence authority |
| Benchmark contract and unfinished qualification | [ADR 9000](../adr/9000-benchmark-strategy.md), [current plan](../plans/2026-10-03-current-source-remediation.md) |
| External consumer applicability and acceptance | [Current plan](../plans/2026-10-03-current-source-remediation.md) and the Semantica deployment owner |
| Final release procedure | [release checklist](../release-checklist.md) |

## Current work, separated by authority

1. **Consumer contract:** Semantica's IDE historical-query caller still forwards an absolute `CompleteBy` deadline to a synchronous blocking path that rejects it. Taskmesh's working tree now provides a separate `run_blocking_response_by`/`run_cpu_response_by` caller boundary with focused custody tests. Semantica source and deployment remain unchanged; that consumer conflict is still open.
2. **Performance admission:** the working-tree validator now compares every measured attempt's producer lag with its hash-bound per-rate control budget. Focused rejection/boundary tests passed. Actual quiet-host series and consumer budgets remain uncollected, so no performance claim is qualified.
3. **Deployment adoption:** D1–D9 applicability, real ingress/planner/wire topology, final-source pinned-pair execution and owner decision remain in the [current plan](../plans/2026-10-03-current-source-remediation.md). Earlier focused pinned-pair tests do not close them.
4. **SDK and maintainability:** the working tree reexports facade plan errors, removes two deprecated Rayon panic constructors, and splits the three large source modules by their existing responsibilities. Consumer-MSRV passed; semver adjudication and final-source CI remain distinct. The default production `expect` sites were classified by reachability in the [current plan](../plans/2026-10-03-current-source-remediation.md); no caller-controlled panic was established.
5. **Deep proof and release:** the required 16-gate merge check is active. The seven costly nightly gates, Linux-only comparison, release adjudication and final release receipt are separate work under the [release checklist](../release-checklist.md).

The ignored root `receipt*.json` set observed in this checkout named
`cc5b256`, not the review baseline. Those three files were moved without
changing their bytes to `target/verification/historical/cc5b256704a1826e7dde36e2c4b127c6cba8aabf/`.
Use a receipt validator
with the intended `--expected-head`; a visible PASS string is not current-source
authority. Keep implementation, focused diagnostics, exact-source CI,
performance, release, and deployment decisions separate.

Taskmesh is a library-only repository; it does not own a deployed binary. An
application's activation and worker topology belong to its consumer owner.
