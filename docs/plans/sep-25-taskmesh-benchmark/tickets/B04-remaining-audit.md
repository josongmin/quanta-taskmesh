# Benchmark remaining audit — 2026-09-26

Audited source: `c5b5282a919eaa77d02a71577b8d8b1cdd8b9812`. During the audit,
HEAD moved to `52f7c008e7ad761e9efb67bedd357c50551aa628`; that commit changes only
ingress test formatting. Benchmark source is unchanged. This audit does not
qualify the engine or its performance.

## Implementation checkpoint — 2026-09-26

The two reproduced acquisition defects below are repaired with focused regressions.
Recorder bundle/control-budget schema v2 adds whole-probe duration sensitivity;
it does not invent minimal-mode request p99. Rust probe artifact publication is
now atomic and create-only. Cohort SLO lookup preserves classes containing `/`
instead of treating the first slash as a path separator. Local schema v3 covers cancel/deadline/drop/Snapshot,
and Criterion has active IO release-to-settled drain with ownership asserted
before timing.

`host_build.py`, `host_study.py` and `host_admission.py` now implement frozen Git
source/repeated cold rebuild custody, complete planned-attempt accounting, and a
separate full-host measured-series admission path. The latter reconstructs raw
runs, recomputes control budgets, reexecutes a cold rebuild and exact-source
correctness oracle, and checks every intended-arrival cohort and independent run.
It reports highest **tested** passing rate and conditional p99 rank intervals.
Sampled CPU per injection-cohort success is retained only as a whole-trial lower
bound; exact CPU/resource-efficiency comparisons remain unsupported.

Measured admission also rejects unoptimized/debug/test executables based on the
actual digest-bound Cargo artifact profile. Oracle test-profile flags are matched
to optimization/debug/overflow semantics of the measured build.

Validation checkpoint at clean `065f6be`: all 218 benchmark Python tests passed;
active IO drain Criterion smoke and local v3 real structural receipt passed.
Two cold builds plus a third actual rebuild produced the same host executable
SHA256. A two-attempt collector smoke used that executable and retained complete
accounting. All these artifacts are diagnostic, not a measured performance series.
Follow-up profile/build/admission checks passed 27 focused Python regressions;
strengthened Rust assertions passed all 6 local/artifact tests. Semgrep passed
with 175 scanned Rust files and zero findings; Python lint, formatting and gate
inventory passed. This checkpoint is not full current-HEAD CI.

Operational contract and remaining real inputs:
[host-series workflow](../../../benchmarks/host-series.md).

**Still open:** actual frozen B00 values, quiet-host measured series, H7 consumer
profile, semantically equivalent peer and external independent rerun; separate
repeated admission lanes for local/closed-loop/composite/Rayon claims. These are
not marked complete by implementation or focused tests. Current-HEAD CI remains
separate from these owner-local checks.

## Historical reproduced code defects (repaired)

### P1 — Build-to-run identity discontinuity

- **Purpose:** Prevent a binary built under source A from receiving source B's
  execution provenance when source changes between the post-build check and
  process launch.
- **Evidence:** `host_run.py` checks identity immediately after build, then
  seals the binary and obtains a new `start_identity` without comparing it to
  `build_start_identity`. The same sequence exists in `generator_run.py`, which
  also owns the minimal-recorder CLI. An orchestration reproduction with
  identity sequence A/A/B/B/B returned exit 0 and `complete` provenance naming
  B. Build, subprocess and raw analysis were mocked to isolate this boundary;
  this is not a real host receipt.
- **Files:** `tools/bench/{host_run,generator_run}.py`, focused acquisition tests.
- **DoD:** Compare the pre-launch identity to the frozen build identity;
  reject drift before launching any runner. Preserve an explicit rejection
  reason without publishing complete provenance. Cover full, generator and
  minimal paths. Keep immutable checkout/build custody as a separate proof:
  an equality check cannot detect a source change followed by restoration.

### P1 — Indexed pair order is not tied to chronological execution

- **Purpose:** Ensure paired control observations follow their declared order
  in actual process windows.
- **Evidence:** The sampler verifier checks indexed modes and sorted-window
  nonoverlap but does not require windows to be chronological by index. A
  synthetic resource study with indexed modes `on/off/off/on` and chronological
  modes `on/on/off/off` passed `host_sampler.verify_bundle`. Actual resource
  validation ran; the constituent raw verifier was mocked. Sequential normal
  acquisition produces ordered windows, but replay verification accepts the
  malformed study. `host_controls.py` sorts constituent windows similarly.
- **Files:** `tools/bench/{host_sampler,host_controls,host_aa,host_observer,host_recorder}.py`
  as needed, their existing tests.
- **DoD:** Bind each study's index/member/mode to its epoch window. Reject
  reversed or permuted acquisition order, enforce both arm order and distinct
  run windows, and retain the invalid bundle reason. Cover the sampler and
  audit the same boundary for A/A, Snapshot and recorder; allow serial studies
  to execute in any order relative to each other.

## Historical implementation and measurement gaps

| Priority / owner | Current source evidence | Required completion |
|---|---|---|
| P1 / B04 recorder measurement | `host_control_assess.recorder_response_effect` compares only responded/intended counts. `MinimalHostRun` has no caller response timestamps. Equal eventual response counts cannot establish a bound on recorder latency distortion. | Define a recorder-cost estimand that minimal mode can independently supply, add its raw/validation fields and frozen budgets, and demonstrate sensitivity with a known recorder perturbation. Do not invent minimal-mode p99. |
| P1 / B04 measured admission | `host_perf.verify_receipt(require_performance=True)` unconditionally rejects after preliminary checks; the control assessor always reports `UNQUALIFIED`. | A digest-bound, revalidated measured-control contract plus independently proven build custody and the frozen B00 contract must feed a separate performance admission path. Caller booleans or a copied budget-pass report cannot unlock it. |
| P1 / B04 build custody | Acquisition retains commands, feature identities, executable digests and endpoint source identities. These are controller statements rather than an independent source-to-binary proof. | Retain exact source/lock/toolchain/build configuration and logs from a frozen checkout; independently rebuild or attest the artifact before admission. |
| P2 / B03–B04 study accounting and analysis | `host_compare.py` accepts a caller-supplied list of successful pairs and computes descriptive SLO-goodput differences. It retains sampled resource lower bounds, but has no expected-attempt ledger, exclusion accounting, qualified capacity verdict or resource-efficiency estimator. | Version the planned and attempted run population, retain every invalid/excluded attempt, bind exclusions to the frozen contract, and add only the metrics supported by the acquired populations and matching windows. Sampled CPU delta is not total CPU-per-success. |
| P2 / B02–B03 feature envelope | Local scenario fields contain no cancel/drop/deadline options and reject Snapshot. H8 measures quiescent drain only. Closed-loop and local/composite lack a qualified repeated comparison lane. | Implement only missing fixtures/modes required by the declared claim, with separate raw schemas, validators and receipts. Active-work teardown must start with owned work; a quiescent drain cannot substitute. Product correctness tests remain their existing owners. |
| B00/B03/B06 / evidence | The claim contract still has null SLO/completion floor/duration/repetitions/MDE/precision, an empty rate grid and peer list, and missing H7 profile provenance. | Pilot and freeze the contract, acquire repeated quiet-host rate/recovery/soak measurements, supply a privacy-safe consumer profile, and qualify a named peer only on proven equivalent semantics with an independent rerun. These are not engine code defects. |

## Proof and documentation boundary

- Earlier change-set checks: 88 affected tests on the working checkout and 42
  focused regressions at clean `c5b5282`; lint and formatting passed. These are
  correctness checks, not performance calibration or full current-HEAD CI.
- `bench-results/receipts/clean-f7834d7/` exists locally (about 42 MB), is ignored
  by Git and contains older structural diagnostics. The scenario index's
  “no repository-retained receipt” wording must be read as no Git-tracked
  receipt, not no local custody. It supplies no current-HEAD performance proof.
- No product-engine execution defect was established by this benchmark audit.
  Unexamined engine paths are outside this conclusion.

Next: validate the newly implemented acquisition/admission workflow on frozen
source, then pilot and freeze B00 and acquire an actual measured series after the
required workload/host inputs exist. Do not substitute historical receipts.
