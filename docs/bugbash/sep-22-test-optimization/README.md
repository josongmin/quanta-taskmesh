# Test optimization audit — 2026-09-22 baseline

Audit baseline: `main` at `7023e945e1c9b3e7e7cca1b26f7af8467df6ab60` on 2026-09-22.

The table below records issues reachable on that source snapshot. A
duplicate-looking test is not actionable unless it either weakens an oracle, leaves a behavior
uncovered, can hang, or has measured avoidable cost.

## Baseline findings

| ID | Priority | Owner | Evidence | Required change |
| --- | --- | --- | --- | --- |
| TO-01 | P0 | `crates/taskmesh/tests/runtime_cancel_timeout.rs:10-23,126-176` | The fixture sets both class inflight and blocking capability to one. The holder saturates both, so the second submission cannot distinguish class saturation from the blocking-pool condition named by the test. | Use class capacity at least two and blocking capacity one. Observe a queued waiter while class capacity remains available and the blocking capability is occupied, then cancel it promptly. |
| TO-02 | P1 | `cancellation_policy.rs:41-98`; `deadline_cancel.rs:563-588,613-650` | Four tests use 20-50 ms sleeps to infer that work started before cancellation. On a delayed runner the token can fire pre-submit, exercising a different branch. | Add explicit started/enqueued handshakes and cancel only after the intended state is observed. Keep bounded outer waits. |
| TO-03 | P1 | `e2e_scenarios.rs:260-312` | Holder start and first saturation are inferred with 50/80 ms sleeps. The retry loop permits 1,000 x 5 ms before failure. | Signal holder start and first `CpuSaturated` explicitly, then release. Replace the arbitrary retry count with a bounded completion timeout. |
| TO-04 | P2 | `e2e_scenarios.rs:358-366`; `e2e_chaos.rs:356-366` | Both tests sleep after all submitted handles have returned before asserting drain. The waits add 110 ms and hide whether completion is synchronously accounted. | Assert drain immediately. If terminal completion intentionally precedes accounting release, stop this optimization and open the production completion-contract owner; do not replace the sleep with polling. |
| TO-05 | P0 | `pyproject.toml:5,11`; `tools/verification/run_generated_mutants.py:14` | The project declares Python 3.9 support and Ruff targets 3.9, but the tool imports stdlib `tomllib`, which exists only in Python 3.11+. | Preserve 3.9 support with an explicit `tomli` fallback/dependency, or raise the project and lint contract to 3.11. Do not mask the producer mismatch with a test skip. |
| TO-06 | P1 | `tools/semgrep/tests/test_rules_fire.py:459-488` | Two tests execute the exact same real-tree Semgrep command. In the current full run they cost 8.99 s and 11.29 s separately. | Share one module-scoped real-scan result; keep the two independent assertions over that result. |
| TO-07 | P1 | `crates/taskmesh-rayon/tests/rayon_smoke.rs:13-19` | Rayon smoke can block until the external runner kills it if accepted work is never executed. | Use `recv_timeout` with a named failure while retaining the returned-value oracle. |
| TO-08 | P1 | `crates/taskmesh-bench/tests/hellgate.rs:140-154` | `hellgate` took 15.94 s; the structural USL test requests 350,000 admit/release cycles but asserts no performance threshold. | Isolate-time the USL test, then reduce/cap structural work only after repeated runs show the three-point fit remains stable. |

## 2026-09-23 source re-audit

At `main@146233665942d75b73e2b724f781be7e105fd7c4` plus the dirty overlay,
the re-audit identified two test-code boundaries, now repaired locally:

- TO-02: first-poll/spawn handshakes are present, but the local cancellation result and stalled
  CPU caller join remain unbounded. See [T02](tickets/SEP22-T02-cancellation-handshakes.md).
- TO-03: the initial saturation is observed, but retry is released before holder completion and
  a loop with `yield_now()` covers that race. The default `OverflowPolicy::Reject`, not queue depth
  alone, causes immediate `CpuSaturated`. See [T03](tickets/SEP22-T03-backpressure-handshake.md).

T02/T03 owner-local tests pass on the repaired dirty source. T04–T08 code changes are also
present on the current source; T05 has a Python 3.9 floor proof, while T04/T08 repeated local
semantic cases passed. The 2026-09-22 rows above are historical findings, not claims that their
original code remains live.

T07's timeout, disconnect, and wrong-value negatives and 20 normal-path runs were later
verified on an isolated owner source. T06's real-source gate and CI missing-binary failure
were rerun. T03–T04 have same-production-source test ablations with median/worst and exact
execution counts; their host-contended worst values are not performance qualification.
All eight tickets have owner-local evidence. T01–T08 are locally verified. T08's
previous 50k→2k workload ladder passed 20/20 on both macOS and Docker Linux, but that
only established cheaper execution of a weak smoke, not a unique correctness oracle. The
current audit removed the real-thread USL smoke entirely: loadgen verifies exact completed
admit/release counts and finite positive throughput, metrics verifies known USL coefficients
and empirical peak, and Criterion owns actual contention timing. The five other Hellgate
tests retain distinct open-loop semantic oracles. See [T08](tickets/SEP22-T08-hellgate-structural-budget.md)
for the explicit coverage loss and focused verification. Clean-source gate/matrix and the
current-source `verify-macos-ci` receipt remain separate qualification work; nightly
and release qualification require explicit authorization.

## Baseline measurements

- `uv run pytest tools -q --durations=40`: `544 passed in 136.17s`.
- `taskmesh-bench` warm test times: `fairness_property` 4.74 s, `hellgate` 15.94 s,
  `inferno` 0.22 s.
- Rust timings were collected under concurrent host load. The
  libtest-reported per-binary times above are retained; contended wall time is not used.

## Removed from the queue

- `wire_formats` anti-vacuity is already closed: tracked seeds exist for `Snapshot`,
  `TopologyConfig`, and `ClassPolicy`; the producer manifest requires all semantic checkpoints;
  qualification rejects zero/partial witnesses.
- TM21-003 physical executor authority is already implemented and has a release proof owner.
- The prior Python failures were environment/stale-snapshot artifacts. The current full suite is
  green; Semgrep, IAI `mktemp`, and the receipt test all passed.
- Cancelled-before-submit variants exercise different substrate/policy/precedence surfaces. Do not
  fold them into one table.
- Snapshot wait helpers and executor fakes have different state predicates and live in separate
  integration-test crates; centralizing them does not reduce execution cost and increases coupling.
- Contract conservation tests, malformed-receipt tests, and Justfile tier tests provide distinct
  layer or diagnostic ownership. Parametrizing them is source compression, not test optimization.
- Do not sample every Nth conservation step, cut fairness seeds, or move coverage to an undefined
  nightly tier without a measured budget and an inventory-backed replacement gate.
- Do not add `slow` markers as a standalone fix. `just py-test` currently selects all markers;
  excluding them without a separate required gate weakens qualification.

Detailed owner evidence: [WS1](ws1-core-runtime.md), [WS2](ws2-engine-model.md),
[WS3](ws3-py-qual.md), [WS4](ws4-contract-bench.md).

## Execution tickets

Implementation order, exclusive ownership, acceptance IDs, negative oracles, exact verification,
and closeout rules are in [`tickets/README.md`](tickets/README.md). `tickets/plan.json` is the
machine-readable mapping; validate it before dispatch with:

The current focused execution record and its dirty-source limits are in
[OWNER-LOCAL-2026-09-23.md](OWNER-LOCAL-2026-09-23.md).

```sh
uv run python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py
```
