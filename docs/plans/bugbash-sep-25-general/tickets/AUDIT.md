# Final audit findings

## Source and evidence state

- The 16/16 PASS macOS CI-profile receipt in `target/verification/macos-gates.json` is bound to audit HEAD `76559c4` and reports 561 workspace tests. HEAD advanced to `29bd8ce` during parallel planning; that receipt does not qualify the newer source.
- The current checkout contains untracked audit/plan documents, so it is not presently a clean qualification workspace.
- No current `target/modelcheck`, TSan, fuzz-campaign, coverage, or mutation artifact was found. `fuzz-check` only compiles/lints fuzz targets; `bench-gate` is an allocation gate; `test-rayon` runs the host lib plus one focused integration case.
- The 104-case static matrix classifies 51 K, 34 P, and 19 G. K means a core observable has a direct fixture, not that every cross-product or nightly proof is complete.

## Findings that require structural work

| ID | Priority | Finding | Evidence | Resolution owner |
|---|---|---|---|---|
| F01 / H30 | P0 | `Builder::build` validates one `CpuExecutor::capabilities()` result, but `TokioRuntime::plan` calls it again and `expect`s a domain. A mutable custom adapter can panic or change capacity authority after build. | `crates/taskmesh/src/builder.rs:179-219`, `crates/taskmesh/src/runtime.rs:317-339`, `crates/taskmesh-contract/src/ports.rs:82-120` | BG25-005 |
| F02 / H34 | P0 after contract decision | Normal requested-stack success and task error are delivered only after owned-runtime teardown. A live `spawn_blocking` child can push caller response beyond `CompleteBy`; terminal deadline/panic paths already answer before teardown. | `crates/taskmesh/src/runtime.rs:630-683`; existing terminal-only fixtures in `hardening_deadline_custody.rs` | BG25-001, BG25-006 |
| F03 / B28 | P1, breaking API risk | `PermitId` and `Ticket` are `u64` aliases and every Governor starts at 1. A foreign raw value that collides locally can act on the local ledger. Lease tokens have authority nonces; raw transition APIs do not. | `crates/taskmesh-engine/src/shared/mod.rs:13-14,193-194`, `engine/governor.rs:115-159` | BG25-001, BG25-004 |
| F04 / D21,D22,D25,H33 | P0 if bytes are untrusted | Derived Serde accepts unknown keys; `parent_awaits` defaults false and `stack_size_bytes` defaults `None`. Typos can disable cycle declaration or change dedicated-stack dispatch to shared blocking. No official strict bytes-to-authority ingress exists. | `crates/taskmesh-contract/src/task.rs:165-225`, config/topology derived Deserialize | BG25-001, BG25-002 |
| F05 / B25 | P1 | Default blocking/CPU paths require Tokio context but do not preflight it before admission. Polling outside Tokio can panic instead of returning a typed error. | `crates/taskmesh/src/runtime.rs:424-430`, `executor/tokio_exec.rs:22-25` | BG25-001, BG25-005 |

## Contract decisions, not automatic engine defects

- B22: parent-stage membership requires a parent-plan registry the engine does not own. Default recommendation: external planner validates membership; engine continues to own declared ancestry/cycle safety.
- B24: the engine preserves classification provenance but cannot prove semantic truth of caller-supplied metadata.
- D17: malformed or foreign-capability preflight may precede closed-admission rejection. The external interface already describes this; align rustdoc/tests rather than changing precedence accidentally.
- D23: unknown future enum variants fail derived Serde decode. This is a consumer version-negotiation policy, not a present execution bug.
- H27: separate runtimes sharing one executor govern only their own submissions unless an explicit shared authority exists.
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
