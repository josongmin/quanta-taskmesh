# Benchmark remaining audit — 2026-09-26

Initial audited source: `c5b5282a919eaa77d02a71577b8d8b1cdd8b9812`. During that audit,
HEAD moved to `52f7c008e7ad761e9efb67bedd357c50551aa628`; that commit changes only
ingress test formatting. Benchmark source is unchanged. This audit does not
qualify the engine or its performance.

## Additional boundary audit — base `f41862d`

The following reachable defects are repaired in this change set. These are
acquisition/admission defects, not demonstrated product-engine defects.

| Boundary | Reproduced failure / risk | Implementation and DoD |
|---|---|---|
| Attempt execution | An unbounded `subprocess.run` can prevent terminal accounting indefinitely; a successful parent can leave a probe executing. | `host_study.py` plan/ledger v2 requires positive integer `timeout_seconds`; reuse `tools/process_supervisor.py` for owned-group cleanup and bounded capture. Real subprocess regressions cover a hung parent/child and a successful parent with a surviving child. Partial output and failed attempts remain in the ledger. |
| Interruption and input custody | Further planned attempts must not launch after interruption; absolute/traversal/symlink input paths contradict the documented plan-root boundary. | Retain explicit failed/not-launched terminal events after SIGINT/SIGTERM; reject escaping/nonregular inputs before execution; reject symlink metadata and unplanned study-root artifacts. Regression covers missing/invalid timeouts, interruption, absolute/traversal and symlink escape. |
| Cold build and oracle execution | These independent proof producers were also unbounded. | Apply the same supervisor with 1,800-second limits. Preserve stdout/stderr and execution metadata after timeout, interruption or incomplete process custody. Do not publish witness/admission success. |
| Cargo configuration custody | Only Cargo-home config was hashed; Cargo also discovers ancestor config outside the frozen archive. | Build witness v2 binds source/ancestor/home `config` and `config.toml`; changes reject. v1 witnesses need reacquisition. Capture Cargo build/target/profile environment and workspace wrapper/toolchain selectors. |
| Effective build semantics | Cargo's example profile can claim optimization while rustflags override it, or governed libraries use different per-package profiles. Oracle environment flags alone do not prove its actual profile. | Measured admission rejects custom rustflags/compiler/wrappers/config `[env]` injection. Check four governed library profiles in both cold logs; check three governed libraries and four test targets in actual oracle Cargo JSON. Optimization/assertion/overflow semantics must match the measured probe. |
| Validated byte custody | The admission rereads raw for tail analysis after typed validation, without comparing that read to the validated digest; validated artifacts also need a direct ledger link. | Bind every validated run artifact to its sealed ledger digest, then hash the exact raw bytes used for tail statistics. Regressions reject differing typed/ledger populations and an altered second read before any rebuild/oracle. |
| Local validator / MSRV | Final CI at `71f0edb` reproduced `manual_checked_ops` and use of `Option::is_none_or` (stable since 1.82) under the declared 1.81 MSRV. | Use `checked_div(...).unwrap_or(0)` and `map_or(true, ...)`, preserving zero-cadence and invalid-timestamp semantics. Confirm through the benchmark Clippy gate and local validator tests. |
| Statistical model | Exchangeability alone does not justify a binomial rank interval for correlated samples. | State **iid within-run success latencies, unverified** in output and workflow. The interval remains conditional; no empirical correlation/stationarity proof or universal precision claim is inferred. |

Owner files: `tools/bench/{host_study,host_build,host_admission,host_perf}.py`,
their existing test modules, `crates/taskmesh-bench/src/local_host.rs`, and
`docs/benchmarks/host-series.md`. No public engine
API or runtime inventory is changed. Tests of synthetic admission orchestration
remain synthetic; real subprocess cleanup tests prove only process ownership.
An initial `561a00e` cold build and CI attempt were intentionally interrupted
for this final byte-custody repair; they provide no success proof. The
authoritative CI receipt, if produced, must match the exact clean commit;
the historical checkpoints below are not substitutes.

Focused owner checks on the final working change: 78 Python tests passed across
`test_host_study.py`, `test_host_build.py` and `test_host_admission.py`; Ruff and
`git diff --check` passed. These are working-change correctness checks. The
current clean-source CI result belongs to `target/verification/macos-gates.json`
and must be validated against its exact HEAD; build/oracle artifacts are
separate and never establish a measured performance result.

CI at `71f0edb` failed Clippy on the two local-validator issues above; downstream
gates were not run. Separate changes to `docs/bugbash/sep-21/` and `tools/pm/`
also made the shared checkout dirty during that attempt. Preserve those other
owner changes and run final qualification in an isolated clean checkout. The
failed/interrupted attempts cannot supply final CI proof.

### Final local verification disposition

- Code checkpoint: `381311e1089d40e9c62a147513ebc45b4388e4bc`.
- The repaired benchmark crate passed `cargo clippy --locked -p taskmesh-bench
  --all-targets -- -D warnings`. `host_local` executed 5/5 and
  `artifact_publish` 1/1 passing integration tests on the shared checkout.
  A prior `--lib local_host` invocation selected zero tests and supplies no proof.
- Isolated clean checkout: `/tmp/taskmesh-final-381311e`. Its CI receipt is
  `target/verification/macos-gates.json` under that checkout. Seven gates passed:
  fmt, inventory, Python lint, architecture, semgrep, deny and full Clippy.
  The Rust `test` gate was intentionally interrupted after 602 seconds during
  the contested-host attempt; it is `FAIL`/interrupted, not an asserted engine
  failure. The eight remaining gates are `NOT_RUN`. **CI is NOT_QUALIFIED.**
  The receipt reports no source-stability problems.
- Optimized frozen-build attempts at `/tmp/taskmesh-build-561a00e` and
  `/tmp/taskmesh-build-71f0edb` were interrupted for final repairs, retain failed
  build logs and have no completed witness. **Current optimized cold-build /
  actual oracle-profile end-to-end proof remains open.** Historical default
  build/oracle results below cannot satisfy the new v2/profile boundaries.
- Next verification requires a clean source checkout and available host resources:
  complete the CI profile, acquire optimized v2 witnesses, execute the third
  rebuild and actual Cargo-JSON oracle check, then exercise the collector with
  those witnesses. This remains separate from B00/H7/peer measurement inputs.
- All changes were committed locally. Other-owner dirty `AGENTS.md`,
  `docs/bugbash/sep-21/` and `tools/pm/` edits were preserved. No push, mutation
  campaign, release qualification or performance-series admission was performed.

### Remaining completion requirements

1. **B00 / measurement owner:** supply actual per-class SLOs, completion floor,
   sampling precision/model, duration and pilot-selected absolute rate grid;
   freeze scenario and control-budget digests before measured attempts.
   DoD: validated frozen contract and complete repeated ledger; null/empty B00
   values and a smoke fixture cannot be promoted to capacity evidence.
2. **Host / measurement owner:** declare a quiet host and collect measured
   controls, repeated rate/recovery/soak populations under one source/build/
   boot/topology identity. DoD: artifact-backed admission with all denominators,
   failures and scope intact. Shared-host smoke/build/CI does not meet this DoD.
3. **H7 / consumer owner:** provide a privacy-safe trace or consumer profile with
   path/class mix, work distributions, burst/cancel/deadline mix and topology.
   DoD: source identity, transformation and trace digest; invented inputs remain
   synthetic and cannot establish consumer representativeness.
4. **Peer / comparison owner:** select a maintained peer and prove its semantic
   intersection, then obtain matched measurements and an external rerun.
   DoD: equivalent work/admission/queue/caller/worker-custody contract plus
   independent receipts. Raw Tokio/semaphore/Tower controls do not establish
   whole-engine superiority.
5. **Additional mode owners:** repeated local/closed-loop/composite/Rayon
   admission remains unimplemented. Activate it only with a declared claim and
   a separate mode-specific estimand/contract; reuse collector/build custody,
   preserve each raw validator and avoid closed-loop overload-p99 claims.
   DoD: mode-specific repeated raw population and independently passing budgets;
   structural receipts do not close this implementation/measurement gap.

Items 1–4 require measurement or external input, rather than fabricated code
defaults. Item 5 is an explicit scope-extension implementation gap. No industry
SOTA performance claim is admitted by this change set.

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
inventory passed. The follow-up code checkpoint is `acc9297`. At that checkpoint, the four named
engine oracle suites also passed 45/45 with test optimization level 3, debug
assertions off and overflow checks off. This is focused source correctness
proof; it neither admits a measured series nor supplies full current-HEAD CI.

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
