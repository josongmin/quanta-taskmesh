# Current audit findings

## Source and evidence state

- The original audit was frozen at `76559c4`; its receipt remains historical evidence for that source only.
- Executor authority fixes are recorded in `bf03efc` and `3e57f1a`. Deadline response and centralized runtime-context preflight are recorded in `74a18f7`.
- `just dev` passed after `74a18f7`'s source changes: 496/496 tests, Semgrep 0 findings, crate-boundary check, and gate-inventory validation. The exact final commit still requires a fresh `verify-macos-ci` receipt.
- No new model-check, TSan, fuzz-campaign, coverage, or mutation claim is made by this audit. `fuzz-check` only compiles/lints fuzz targets; `bench-gate` is an allocation gate; focused or `dev` results are not release qualification.
- The 104-case static matrix classifies 51 K, 34 P, and 19 G. K means a core observable has a direct fixture, not that every cross-product or nightly proof is complete.

## Source-backed findings

| ID | Status | Finding | Evidence | Owner |
|---|---|---|---|---|
| F01 / H30 | RESOLVED | Runtime planning, debug, and public accessors use the one executor descriptor accepted by `Builder::build`; the adapter is not re-queried. | `bf03efc`; `hardening_executor_protocol::the_validated_executor_descriptor_is_frozen_for_every_runtime_use` | BG25-005 |
| F02 / H34 | RESOLVED | Requested-stack `CompleteBy` now bounds caller response even when owned-runtime teardown outlives a ready success or task error. The late result is discarded while the worker retains custody through teardown. | `74a18f7`; `hardening_deadline_custody::complete_by_discards_success_and_task_error_when_teardown_misses_the_response_deadline` | BG25-006 |
| F03 / B28 | OPEN — contract/API decision | `PermitId` and `Ticket` are `u64` aliases and every Governor starts at 1. A foreign raw value that collides locally can act on the local ledger. Lease tokens have authority nonces; raw transition APIs do not. | `crates/taskmesh-engine/src/shared/mod.rs`, `crates/taskmesh-engine/src/engine/governor.rs` | BG25-001, BG25-004 |
| F04 / D21,D22,D25,H33 | OPEN — deployment boundary | Derived Serde accepts unknown keys; defaults can alter cycle or dispatch declarations. Whether untrusted bytes reach these DTOs is an external ownership fact. Add strict ingress only after identifying that boundary. | `crates/taskmesh-contract/src/task.rs`; config/topology deserializers | BG25-001, BG25-002 |
| F05 / B25 | RESOLVED | Tokio-backed blocking/CPU and timer/local paths now validate required runtime services before admission. Already-decided cancel/deadline verdicts retain precedence and failed preflight leaves no permit or ticket. | `bf03efc`, `74a18f7`; `hardening_executor_protocol::default_tokio_dispatch_without_a_runtime_is_rejected_before_admission` | BG25-005, BG25-006 |
| F06 / D17 | RESOLVED | Closed admission was documented as preceding every direct preflight, while malformed specs and foreign capabilities actually reject first. Rustdoc now states the real order and an exact side-effect-free fixture covers each public intake path. | `1c7b8ac`; `hardening_close_admission::closed_preflight_precedence_is_explicit_and_side_effect_free` | BG25-003 |

## Contract decisions, not automatic engine defects

- B22: parent-stage membership requires a parent-plan registry the engine does not own. Default recommendation: external planner validates membership; engine continues to own declared ancestry/cycle safety.
- B24: the engine preserves classification provenance but cannot prove semantic truth of caller-supplied metadata.
- D23: unknown future enum variants fail derived Serde decode. This is a consumer version-negotiation policy, not a present execution bug.
- H27: separate runtimes sharing one executor govern only their own submissions unless an explicit shared authority exists. `3e57f1a` proves both per-runtime bounds and the aggregate physical executor bound.
- H35: `run_local` owns the submitted root, not arbitrary detached local or ambient Tokio children.

## Proof gaps without a confirmed source defect

- Compound blocker precedence across class, capability, CPU, and measured memory.
- Release/promotion/claim/abandon/timeout/reap combined history.
- WFQ/DRR cancellation with cross-pool service and promotion-budget continuation.
- Multiple drain waiters with reentrant or panicking wakers.
- Stage release, reconcile, promotion, and sweep in one independent memory ledger.
- Differential/model-check coverage of memory, fairness, child scope, wakers, reap, and promotion budget.
- Real public-host open-loop offered/terminal/unanswered/custody accounting.
- Full Rayon-feature integration collection.

These are not pre-labelled code defects. Each ticket must first build a deterministic counterexample with an independent oracle; modify production code only when that fixture disproves the accepted contract.
