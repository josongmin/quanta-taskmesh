# Taskmesh operating kernel

This is a navigation and current-state index. It does not redefine API,
governance, gate membership, or release policy. Follow the owning source and
documents below when they change.

The [2026-10-03 audit](../audits/2026-10-03-current-source-final.md)
records its review baseline and candidate evidence. The later `main` CI scope
is recorded below.

## Source and evidence snapshot

- The 2026-10-03 review baseline was `main@c4bcb2f`. PR #9 subsequently
  merged the Taskmesh host/API, measured-admission, and maintainability changes
  into `main@f81b138`. The [audit](../audits/2026-10-03-current-source-final.md)
  records candidate source identities; its receipts do not qualify later edits.
- [PR #11](https://github.com/josongmin/quanta-taskmesh/pull/11) merged into
  `main@85f0042` on 2026-10-05. The post-merge CircleCI `ci` profile reported
  16/16 PASS for that clean source. Its receipt applies to `85f0042` only;
  later source changes require a fresh exact-source check.
- `.circleci/config.yml` owns the new bounded automatic PR/main source check.
  The former `pr-ci.yml` is manual-only in source. The CircleCI project trigger,
  auto-cancel setting and GitHub required context are live external settings:
  inspect them as described in [CircleCI operations](../ci-circleci.md).
  Full `ci.yml`, `bench.yml`, and `release.yml` remain manual-only in source.
  `nightly` is a high-cost profile, not a schedule.
- Nightly, Linux release, performance and Semantica consumer qualification
  remain OPEN after the bounded CI migration.

## Authority map

| Question | Owner |
|---|---|
| Live engine vs documented knobs | [engine status](engine-status-v1.md), `crates/taskmesh-engine/src/features/fairness/scheduler.rs` |
| Crate boundaries and public API | [ADR 0001](../adr/0001-hexagonal-feature-sliced-architecture.md), [library spec](../taskmesh-library-spec.md), `crates/taskmesh-contract/src` and `crates/taskmesh/src/lib.rs` |
| Task/stage/wire behavior | [external interface](../taskmesh-external-interface.md), [ADR 0004](../adr/0004-sep-25-ingress-plan-identity-and-wire.md) |
| Executor, deadline, response and capacity custody | [ADR 0005](../adr/0005-sep-25-execution-response-and-custody.md), host and engine source |
| Gate membership and commands | `tools/gates/required.json`, `tools/gates/inventory.json`, `Justfile`; [ADR 0006](../adr/0006-source-bound-verification-authority.md) defines evidence authority |
| Benchmark contract and unfinished qualification | [ADR 9000](../adr/9000-benchmark-strategy.md), [current plan](../plans/2026-10-03-current-source-remediation.md) |
| External consumer applicability and acceptance | [Current plan](../plans/2026-10-03-current-source-remediation.md) and the Semantica deployment owner |
| Final release procedure | [release checklist](../release-checklist.md) |

## Current work, separated by authority

1. **Consumer contract:** Semantica's IDE historical-query caller still forwards an absolute `CompleteBy` deadline to a synchronous blocking path that rejects it. Taskmesh now provides a separate `run_blocking_response_by`/`run_cpu_response_by` caller boundary with focused custody tests. Semantica source and deployment remain unchanged; that consumer conflict is still open.
2. **Performance admission:** the validator compares every measured attempt's producer lag with its hash-bound per-rate control budget. Focused rejection/boundary tests passed. Actual quiet-host series and consumer budgets remain uncollected, so no performance claim is qualified.
3. **Deployment adoption:** D1–D9 applicability, real ingress/planner/wire topology, final-source pinned-pair execution and owner decision remain in the [current plan](../plans/2026-10-03-current-source-remediation.md). Earlier focused pinned-pair tests do not close them.
4. **SDK and maintainability:** Taskmesh reexports facade plan errors, removes two deprecated Rayon panic constructors, and splits the three large source modules by their existing responsibilities. Default/Rayon consumer-MSRV passed in the bounded `main@85f0042` CI; semver adjudication remains open. The default production `expect` sites were classified by reachability in the [current plan](../plans/2026-10-03-current-source-remediation.md); no caller-controlled panic was established.
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
