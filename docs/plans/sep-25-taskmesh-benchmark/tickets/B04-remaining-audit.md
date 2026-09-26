# Benchmark qualification — open work only

- Status: **OPEN / performance UNQUALIFIED** at `cbf9764` (2026-09-27)
- Decisions and completed owner-local implementation:
  [ADR 9000](../../../adr/9000-benchmark-strategy.md)
- Original audit and completed code history: Git `cbf9764`, this path

The benchmark harness, custody/inspection hardening, bounded standalone
execution, complete-population B07 recovery diagnostic, and local structural
oracles are implemented. Those results do not satisfy any item below. No
product-engine defect is inferred from an unmeasured performance claim.

| Owner / priority | Open action | Completion evidence |
|---|---|---|
| BG25-012 / CI, required for current source | Run `just dev` and `just verify-macos-ci` on a frozen clean HEAD, then validate its receipt with that exact `--expected-head`. | Every applicable required CI gate PASS, no required NOT_RUN/FAIL, stable HEAD/tree/path digest and retained receipt. Earlier 16/16 receipts and dirty owner runs are source-specific history. |
| Build/oracle owner / P1 | Independently bind an optimized, frozen source/lock/toolchain/build configuration to the binary and actual oracle-profile E2E. | Frozen archive/config/logs, reproducible build or separate attestation, retained binary identity and real validator execution. Controller-stated digests or a debug smoke alone do not prove source-to-binary custody. |
| B00 measurement owner / input | Supply actual class/path SLOs, completion floor, sampling precision, duration, repetition count, MDE and absolute rate grid; preregister exclusions and pilot-derived control budgets. | Validated non-null claim contract and scenario/control digests frozen before measured attempts. Smoke defaults are not consumer budgets. |
| B04 host owner / measured admission | Acquire quiet-host balanced generator headroom, A/A, recorder/Snapshot and sampler controls and repeated rate/recovery populations under fixed source/build/boot/topology. Implement a separate measured admission path; `host_perf.py --require-performance` remains closed. | Raw intended/terminal/custody denominators, all failed attempts, source-bound budgets, independent build custody, run-level uncertainty and a qualified verdict. Control-budget PASS alone remains `UNQUALIFIED`. |
| B04 recorder owner / P1 | Define a minimal-mode recorder-cost estimand that can expose response distortion without inventing a minimal-mode p99. | Separate validated raw field/population, frozen budget and sensitivity to a known perturbation. Equal eventual response counts are insufficient. |
| B03–B04 study owner / P2 | Version planned and attempted runs, exclusions and resource populations. Extend the comparator only where the collected data supports the metric. | Complete attempt ledger, frozen exclusion rules, matched windows; sampled maxima remain lower bounds and one CPU sample is not CPU-per-success. |
| H7 consumer owner / input | Supply a privacy-safe representative path/class/body/burst/cancel/deadline profile and topology. | Exact source, transformation and trace digest. Synthetic fixtures cannot establish consumer representativeness. |
| B06 comparison owner / conditional | Select a maintained peer with an explicit semantic intersection, matched host measurements and an external rerun before an industry claim. | Equivalent admission/queue/deadline/custody scope and independent receipts. Raw Tokio, Semaphore and Tower are narrower mechanism controls. |
| Additional mode owners / conditional | Add repeated local, closed-loop, composite, requested-stack or Rayon admission only for a declared mode-specific claim. Automatic RSS slope gates or recovery SLOs likewise require frozen consumer limits. | Separate estimand, bounded raw/controls, mode-specific validators and repeated receipts. Closed-loop completion tails cannot join open-loop intended-arrival p99. |

Mutation, modelcheck, TSan, fuzz, coverage, IAI, nightly/release and deployment
are separate authorities under [ADR 0006](../../../adr/0006-source-bound-verification-authority.md)
and the [release checklist](../../../release-checklist.md). The 600-second B07 H2
diagnostic and post-commit H5 smoke are functional owner-local observations,
not a longitudinal leak guarantee or performance result.
