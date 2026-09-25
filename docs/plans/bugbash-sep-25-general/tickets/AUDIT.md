# Current audit findings

## Source and evidence state

- Final re-audit at clean committed `da5356b253fc69450e98869a2c94e4a3e770747e` verified `target/verification/macos-gates.json`: macOS CI profile 16/16 required PASS, 0 required NOT_RUN/FAIL, stable HEAD/tree/path digest. Subsequent tracked ticket edits require a new receipt; the plan and 104-row validators still prove static structure only.
- The original audit was frozen at `76559c4`; its receipt remains historical evidence for that source only. Current qualification is regenerated only after the implementation is committed and the source is clean.
- Executor authority fixes are recorded in `bf03efc`, `3e57f1a`, and `0b608c3`. Deadline response and centralized runtime-context preflight are recorded in `74a18f7`.
- `just dev` passed after `74a18f7`'s source changes: 496/496 tests, Semgrep 0 findings, crate-boundary check, and gate-inventory validation. The completed fixes were then rerun through `verify-macos-ci`; the generated receipt remains valid only while its clean source HEAD, tree, and digest match the checkout.
- No new model-check, TSan, fuzz-campaign, coverage, or mutation claim is made by this audit. `fuzz-check` only compiles/lints fuzz targets; `bench-gate` is an allocation gate; focused or `dev` results are not release qualification.
- The 104-case static matrix classified 51 K, 34 P, and 19 G at audit time. `scenario-evidence.json` preserves that origin and now binds every row to a selected deterministic fixture; nightly proof remains separate.

## Source-backed findings

| ID | Status | Finding | Evidence | Owner |
|---|---|---|---|---|
| F01 / H30 | RESOLVED | Runtime planning, debug, and public accessors use the one executor descriptor accepted by `Builder::build`; the adapter is not re-queried. | `bf03efc`; `hardening_executor_protocol::the_validated_executor_descriptor_is_frozen_for_every_runtime_use` | BG25-005 |
| F02 / H34 | RESOLVED | Requested-stack `CompleteBy` now bounds caller response even when owned-runtime teardown outlives a ready success or task error. The late result is discarded while the worker retains custody through teardown. | `74a18f7`; `hardening_deadline_custody::complete_by_discards_success_and_task_error_when_teardown_misses_the_response_deadline` | BG25-006 |
| F03 / B28 | RESOLVED — qualification pending | `PermitId` and `Ticket` carry private Governor authority plus local sequence. Same-sequence foreign release/advance/claim/abandon return typed negative outcomes with state/callback 0. Counter exhaustion rejects without wrap/reuse. | `crates/taskmesh-engine/tests/identity_authority.rs` | BG25-004 |
| F04 / D21,D22,D25,H33 | PARTIAL — library boundary implemented; deployment adoption OPEN | `taskmesh::ingress` provides bounded strict task/config byte entrypoints. It rejects oversized/deep input, stage overflow, duplicate/unknown nested keys, implicit child wait, and implicit blocking dispatch before runtime promotion. Raw DTO Serde remains compatible, and no in-repo production caller establishes that external untrusted bytes use this boundary. | `crates/taskmesh/src/ingress.rs`; `strict_ingress` | BG25-001, BG25-002 |
| F05 / B25 | RESOLVED | Tokio-backed blocking/CPU and timer/local paths now validate required runtime services before admission. The frozen executor descriptor carries this requirement, so an explicitly installed built-in Tokio adapter cannot bypass it. Already-decided cancel/deadline verdicts retain precedence and failed preflight leaves no permit or ticket. | `bf03efc`, `74a18f7`, `0b608c3`; `hardening_executor_protocol::{default_tokio_dispatch_without_a_runtime_is_rejected_before_admission,explicitly_installed_tokio_cpu_adapter_requires_context_before_admission}` | BG25-005, BG25-006 |
| F06 / D17 | RESOLVED | Closed admission was documented as preceding every direct preflight, while malformed specs and foreign capabilities actually reject first. Rustdoc now states the real order and an exact side-effect-free fixture covers each public intake path. | `1c7b8ac`; `hardening_close_admission::closed_preflight_precedence_is_explicit_and_side_effect_free` | BG25-003 |

## Contract decisions, not automatic engine defects

- B22: parent-stage membership requires a parent-plan registry the engine does not own. Default recommendation: external planner validates membership; engine continues to own declared ancestry/cycle safety.
- B24: the engine preserves classification provenance but cannot prove semantic truth of caller-supplied metadata.
- D23: unknown future enum variants fail derived Serde decode. This is a consumer version-negotiation policy, not a present execution bug.
- H27: separate runtimes sharing one executor govern only their own submissions unless an explicit shared authority exists. `3e57f1a` proves both per-runtime bounds and the aggregate physical executor bound.
- H35: `run_local` owns the submitted root, not arbitrary detached local or ambient Tokio children.

## Closed deterministic proof gaps

- `hardening_admission_matrix` and `hardening_admission_ledger`: compound blocker precedence, independent event accounting, and 0/1/exact/+1 capacity.
- `hardening_queue_history`: release, promotion, claim, abandon, reap, terminal retention, and final quiescence in one finite history.
- `hardening_memory_ledger`: stage release, reconcile, promotion, measured overcommit, and leak sweep against an input-derived ledger.
- `host_open_loop`: offered/terminal/unanswered accounting and caller-response versus worker-custody accounting.
- `hellgate::multiclass_multiseed_population_has_no_unaccounted_request`: multi-class, multi-seed simulator population conservation.
- `hardening_root_child_scope`: unawaited local-child drop and ambient Tokio child lifetime.
- `scenario-evidence.json`: all 104 audit IDs selected by existing CI-profile gates.

Nightly modelcheck, TSan, fuzz, coverage, IAI, and mutation remain NOT_RUN until explicitly authorized. This does not reduce the deterministic 104-row status or upgrade CI proof to release qualification.
