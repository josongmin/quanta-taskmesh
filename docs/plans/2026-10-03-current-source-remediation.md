# Taskmesh remaining work after the 2026-10-03 source audit

- **Status:** Taskmesh candidate CI and PR publication passed at the source
  identities below. OPEN: 0.3 compatibility adjudication, performance evidence,
  the nightly operating decision and external consumer acceptance.
- **Taskmesh source:** audit baseline
  `main@c4bcb2f213c66aac8adc7d3493c5a39917a29e00`; the implementation is
  on `codex/taskmesh-current-source-remediation`. The
  [current-source audit](../audits/2026-10-03-current-source-final.md) records
  the candidate sequence and proof scope. A receipt or hosted result qualifies
  only the exact candidate SHA it names.
- **Semantica:** read-only observation at
  `a961cf26842e77fc6231f16b057626c69cb7897d`; its Taskmesh manifest and
  lockfile still pin `c4bcb2f`, and its blocking adapter still passes
  `with_absolute_deadline` to `run_blocking_with`. Changes in that repository
  are deferred by user direction.

## T1 — exact-source Taskmesh CI and publication (completed for `f2808d3`)

**Owner:** Taskmesh. The implementation was committed on an isolated `codex/`
branch after reviewing the code, documentation deletion and evidence relocation.
Clean `f2808d379b743fc5eaa7e295987c906dd7bc2957` passed all 16 local macOS
CI gates; its receipt validated with `--expected-head`. [PR #9](https://github.com/josongmin/quanta-taskmesh/pull/9)
passed the strict hosted [required check](https://github.com/josongmin/quanta-taskmesh/actions/runs/37105889890)
on synthetic merge `85907fc3fdef2c13962daf186f7f0b6ab2a08d5e` with 16/16
PASS. The branch rule was already configured. These results do not qualify a
later commit: rerun the same exact-source checks for a changed head or merge.
`docs/ssot/README.md` remains a navigation index, and this repository remains
library-only.

## T2 — 0.3 API compatibility adjudication

**Owner:** Taskmesh release. **Prerequisite:** T1 candidate frozen. The
unreleased 0.3 API removes deprecated
`RayonCpuExecutor::new/from_topology`; consumers use typed
`try_new/try_from_topology` and `taskmesh::ext::RayonBuildError`.

1. `just semver-release` reported against immutable 0.2.0 on clean `f2808d3`.
   All four public crates have findings; the Rayon report names the two
   intentionally removed constructors. This producer reports Rust API changes
   but does not decide wire or behavior compatibility. Rerun it for a later
   release candidate.
2. Record the final-SHA human adjudication for Rust API, wire and behavior in
   the release process. The independent consumer-MSRV default/Rayon fixture
   passed on the earlier clean CI candidate and in a focused run after the
   atomic sequence repair, including facade plan errors and typed Rayon
   construction, but does not stand in for release approval.
3. Follow [release-checklist.md](../release-checklist.md). Mutation/nightly,
   Linux and final release qualification are distinct from T1's 16-gate CI.

**Stop:** missing baseline/tool identity, unadjudicated source break or a
changed candidate after the semver report.

## T3 — measured host performance qualification

**Owner:** Taskmesh benchmark. The validator repair now rejects a measured
attempt when its reconstructed `max_producer_lag_ns` exceeds the same rate's
hash-bound `max_host_lag_ns`. A lag-only rejection and equal-to-budget pass are
covered by focused Python tests. This is validation logic, not a performance
result; [ADR 9000](../adr/9000-benchmark-strategy.md) owns the contract.

1. Obtain B00 consumer SLOs, completion floor, precision/MDE, absolute rate
   grid and representative trace. Freeze build/source, workload/oracle and
   per-rate control policy before measurement.
2. Acquire balanced repeated baseline/candidate attempts on a quiet host,
   retaining every raw run, failed attempt, provenance and sealed ledger.
   Evaluate generator/observer headroom and resource sampler distortion.
3. Admit only the complete registered series. Report SLO-goodput, completion,
   class/path latency populations and run-level uncertainty; a diagnostic
   `UNQUALIFIED` comparison or a local validator pass is not a host verdict.

**Stop:** budget not preregistered, incomplete or changed artifacts, lag above
budget, source/topology/workload mismatch, or correctness/drain failure.

## T4 — nightly schedule decision

**Owner:** Taskmesh operations. The seven costly nightly gates are a proof
profile, not a schedule. `ci.yml`, `bench.yml` and `release.yml` are currently
manual-only and disabled in GitHub; no cron exists. `AGENTS.md` requires
explicit current-request authorization before running mutation-bearing
nightly/release commands. The user has been asked to choose among a full
mutation-bearing weekly schedule, a bounded CI-only schedule, and deferral.

If authorized, specify runner budget, concurrency, retention, artifact identity,
failure alert and receipt authority before wiring a cron. A CI-only schedule
must have its own name and denominator; it cannot satisfy the seven-gate
nightly or release receipt. Do not silently enable the disabled full workflows.

## T5 — Semantica consumer contract and D1–D9 adoption (deferred)

**Owner:** Semantica deployment owner, with Taskmesh paired proof. Taskmesh now
provides `run_blocking_response_by`/`run_cpu_response_by` as an absolute
acquisition-to-caller-response boundary. Synchronous `CompleteBy` still
rejects. A started worker retains its lease until it actually terminates.

When this work resumes, change the selected Semantica blocking caller to the
new response-bound API, preserve root invocation identity,
`source`/`reason`, BackgroundOnly role, stack dispatch and error projection,
then pin the new Taskmesh commit in the manifest and resolved lockfile. On one
frozen source pair, test expired, queued, grant/start race, held worker, caller
drop, panic and drain/successor capacity. Keep a negative test for sync
`CompleteBy`.

Record deployment-owner applicability or observed behavior for each boundary:

| Boundary | Required evidence or owner-confirmed not-applicable decision |
|---|---|
| D1–D2 | Actual untrusted bytes ingress and awaited-child call paths. |
| D3–D4 | Selected blocking/requested-stack domain and any external planner. |
| D5–D6 | Opaque handles; caller response, worker custody, capacity and drain after timeout. |
| D7–D8 | Tokio context and any externally shared executor authority. |
| D9 | Actual serialization/version boundary and unknown-variant rejection. |

A Git dependency pin or earlier focused 2/2 consumer tests do not close
D1–D9. Deployment activation and owner acceptance are separate decisions.

## Closed or reclassified at this audit

- Required PR 16-gate check: already configured and strict on `main`; latest
  baseline result passed at `c4bcb2f`.
- M1 validator, S1 facade errors, S3 Rayon constructor source change and the
  Taskmesh side of the response boundary: implemented in the current dirty
  tree; focused checks passed. Their remaining proof is in T1–T3/T5.
- `expect`: default production calls were reviewed against private/validated
  preconditions. No caller-controlled panic counterexample was established.
  Test-only and `test-util` paths are outside that claim. Convert a specific
  newly demonstrated bypass to a typed error, not every invariant assertion.
- R1: host acquisition/synchronous/settlement, engine memory/observation/
  validation/state transitions and fairness tests were split. Parent
  runtime/governor/state files are 860/903/655 lines. This is maintainability,
  not an inferred production defect from line count.
- O2: ignored root `receipt*.json` naming `cc5b256` moved byte-for-byte to
  `target/verification/historical/<source-head>/`. Use `--expected-head` for
  every cited receipt; no automatic false GREEN was demonstrated.
- Broader SEP-27 proposals (typed class builder, generic submission builders,
  config sharing, optional host observation) had no accepted consumer contract
  and are not part of this remediation. Reopen only with a concrete caller and
  crate-boundary decision.

The old Sep-16, BG25, benchmark, CI-stage and SEP-27 narrative plans were
removed. Completed decisions are compressed in
[ADR 0003–0007](../adr/README.md) and measurement policy in
[ADR 9000](../adr/9000-benchmark-strategy.md). Historical scenario mapping
and receipts are preserved under `docs/evidence/`. Sep-21 `plan.json`, tickets
and packets remain at their source-bound paths because release receipt,
finding proof and adjudication tooling consume their exact bytes; they are
release inputs, not the current work queue. Relocation requires a versioned
manifest migration and fresh exact-source proof.
