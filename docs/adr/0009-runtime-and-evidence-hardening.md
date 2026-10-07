# 0009. Runtime and evidence hardening after Sep-25

- Status: Accepted as implemented repository contracts; qualification is separate
- Date: 2026-10-07
- Reviewed source: `e503b6c74c39209bbbafb31f8619e85122a6e714`
- Cleanup baseline and archived narratives: `2123a462764a3d6445fd536a1e5a5f99d019425e`
- Amends: [0003](0003-sep-16-hardening-contracts.md),
  [0005](0005-sep-25-execution-response-and-custody.md),
  [0006](0006-source-bound-verification-authority.md),
  [9000](9000-benchmark-strategy.md)

## Context

Sep-26/27 and Oct-3/4 audit reports repeated implementation and proof state.
Their accepted repairs are consolidated here. Raw receipts retain their original
source tuple and scope; prior focused or isolated CI results do not qualify this
source. Exact old narratives are bound in [document history](../evidence/document-history.json).

## Decision

| Completed work | Structural contract and owner |
|---|---|
| A01, A08 effect/custody | Undelivered admission/claim compensates only dispatch-owned reservations. `abandon` refuses worker-owned leases (`HeldByLease`). Accounting faults are sticky: reject new work, terminalize/wake queued work with typed reasons, and retain live worker capacity until settlement. `crates/taskmesh-engine/src/engine/governor.rs`, `engine/state/transition.rs`. |
| A02, later fairness fixes | Queue identity, capability predecessors and awaited children have common enqueue/removal indexes. FIFO, WFQ and deadline ties use physical lock-committed `queue_order`, independently of permit/ticket allocation order. DRR requires positive quantum (A09). `crates/taskmesh-engine/src/features/fairness`. |
| A03 | Snapshot conservation uses checked arithmetic; overflow and started > admitted reject. `crates/taskmesh-contract`. |
| A04, A07 | Checkpoint is preserved metadata with no automatic hook loop (`Governor::checkpoint_policy`). Strict ingress distinguishes omitted blocking dispatch from explicit null; null rejects. `crates/taskmesh/src/ingress.rs`. |
| A05, Oct-3 response API | Success is finalized after the release fence and deadline/cancellation recheck. Synchronous `*_response_by` covers acquisition through caller response; started workers retain capacity until termination. Requested-stack teardown races caller cancellation/deadline while retaining custody. `crates/taskmesh/src/runtime`. |
| A06, subsequent supervision | Retain waitable leader/process-group custody, binary capture and bounded cleanup even after exit 0/100. Interrupted producers stop launching later work and retain incomplete rows. Environment observation errors fail closed while cleanup continues. Linux zombie leaders require observed thread quiescence. `tools/process_supervisor.py`, `tools/bench/acquisition_process.py`, `tools/release/execution.py`. |
| A10, Sep-26/27 trust | Identity uses actual source bytes/modes and effective compiler/wrapper/linker bytes. Recheck after build/oracle/validator execution. Nonregular ingress rejects before blocking; validation executes private copies of hashed inputs. Bind raw capability inventory, settled conservation and complete planned arm populations. `tools/bench`; details in ADR 9000. |
| A11, A12 | Concurrency memory-release tests assert actual freed units and refused completion-only releases. Fairness properties construct valid production policies and preserve the original 3000-seed denominator. `crates/taskmesh-engine/tests`, `crates/taskmesh-bench/tests/fairness_property.rs`. |
| SDK and CI | Facade exports plan errors and typed Rayon build errors; removed panic constructors require compatibility adjudication. Pin bounded CI compiler independently of consumer Rust 1.81. Preserve 0.2 `AdmissionVerdict` discriminants by appending new variants. the four public crate source trees, `.circleci/config.yml`. |
| PRs #12–27 proof repairs | Process/producer custody, generated discovery, requested-stack cancellation and admitted-deadline fixtures have independent oracles. PR #27 adds child-key-only and child-path-only failure witnesses, partial event/time guards, success-raw field witnesses and observable warmup invariants. `crates/taskmesh-bench/tests/host_composite.rs`, `crates/taskmesh-bench/src/composite_host.rs`. |
| PR #28 procfs race | A disappearing directory entry is omitted only if the PID is confirmed gone or outside the owned group. Held leader, owned member and unreadable ownership remain incomplete. `tools/process_supervisor.py`; independent observations in `tools/gates/tests/test_batch_supervisor.py`. |
| Oct-7 ADR input migration | Release closure requires a safe, digest-bound ADR artifact before reading its section. An internal parent symlink cannot supply closure with `ticket_artifact = null`. The map retains mutable release checkpoint status separately from immutable historical ticket status. `tools/release/receipt.py`, `tools/release/ticket_map.py`. |

## Current source and regression owners

The historical A01–A12 labels above group changes; they are not active tickets.
The following owners were checked directly at the reviewed source.

| Boundary | Source | Regression witness |
|---|---|---|
| Queue indexing and lock-committed ordering | [state](../../crates/taskmesh-engine/src/engine/state.rs), [transition](../../crates/taskmesh-engine/src/engine/state/transition.rs), [scheduler](../../crates/taskmesh-engine/src/features/fairness/scheduler.rs) | `queued_indexes_match_prefix_oracle_across_middle_and_head_removals`, `fifo_and_tied_priorities_select_lock_commit_arrival_before_preallocated_identity`; [fairness reference](../../crates/taskmesh-engine/tests/hardening_fairness_reference.rs) |
| Accounting-fault custody | [Governor](../../crates/taskmesh-engine/src/engine/governor.rs) | [host accounting fault](../../crates/taskmesh/tests/host_accounting_fault.rs), `measured_overflow_terminalizes_unbounded_host_waiter_without_releasing_worker` |
| Response/release fence | [runtime](../../crates/taskmesh/src/runtime.rs), [detached worker](../../crates/taskmesh/src/runtime/detached.rs), [synchronous API](../../crates/taskmesh/src/runtime/synchronous.rs) | [deadline custody](../../crates/taskmesh/tests/hardening_deadline_custody.rs), [drain](../../crates/taskmesh/tests/hardening_drain.rs) |
| Strict null/presence boundary | [ingress](../../crates/taskmesh/src/ingress.rs) | [strict ingress](../../crates/taskmesh/tests/strict_ingress.rs), `nonblocking_dispatch_tag_must_be_absent_even_when_null` |
| Process-group observation | [supervisor](../../tools/process_supervisor.py) | [batch supervisor](../../tools/gates/tests/test_batch_supervisor.py), including disappearing stat, held leader and zombie-thread fixtures |
| Generated proof denominator | [producer](../../tools/verification/run_generated_mutants.py) | [denominator tests](../../tools/verification/tests/test_run_generated_mutants.py), `test_equal_count_identity_mismatches_fail_closed` |
| Composite key/path and success raw | [raw validator](../../crates/taskmesh-bench/src/composite_host.rs) | [composite host](../../crates/taskmesh-bench/tests/host_composite.rs), `composite_failure_raw_rejects_child_key_and_path_independently` |
| ADR release inputs | [map parser](../../tools/release/ticket_map.py), [release evaluator](../../tools/release/receipt.py) | [map/closure regressions](../../tools/pm/tests/test_ticket_map.py), internal parent symlink refusal and independent historical/checkpoint lifecycle |

## Consequences and operational limits

Queue indexes add bounded O(N*k) state with k <= 32; selection still examines
queued candidates and declared cycles. No new pool or unbounded execution queue
is introduced. AccountingFault is fail-stop: allow existing leases to settle,
then replace the Governor. Faulted tickets remain bounded by queue depth until
claim/abandon or Governor destruction.

The Oct-4 queue cost samples excluded fixture creation/cleanup and used an
uncontrolled host. They are source-specific diagnostics, not a performance
qualification. Benchmark execution deadlines are safety bounds, not consumer
SLOs. Regular-file kernel/network stalls and deliberate owner-environment
stripping remain outside the stated supervision contract.

## Compatibility decision input

The immutable 0.2.0 baseline is `39bee682d7daa1efaf1c10993ba6221fd0a90871`.
The source breaks to adjudicate include:

- Contract: `TopologyConfig.physical_domains`, opaque `PlanSource`, changed
  derives, terminal variants and exact child parent identity/builder arity.
- Engine: observation fields, provenance/claim derives, lease-protected
  release/abandon outcomes and removed policy capability methods.
- Facade/adapters: non-unit `BlockingPoolCpuExecutor`, removed Rayon panic
  constructors and changed `try_*` error type; facade reexports need manual review.
- Wire/behavior: legacy valid provenance strings versus invalid identity refusal,
  raw DTO versus strict ingress, deadline/preflight precedence and response/custody
  transitions. Rust semver reports alone cannot adjudicate these surfaces.

[CHANGELOG](../../CHANGELOG.md) retains migration guidance. All 17 decisions in
`tools/release/adjudication.json` remain PENDING; this ADR supplies technical
context and does not approve a version, finding or release.

## Verification boundary

At consolidation, current source contains PR #28 ownership classification and
the PR #27 oracles, including
`composite_failure_raw_rejects_child_key_and_path_independently`. The previous
`e5c209ce` partial mutation miss is historical after that merge. The unchanged
production key-or-path guard remains; new test source alone is not full-denominator
qualification. No mutation campaign, clean-source CI, performance series or
consumer execution was run for this document consolidation.

The Oct-7 code-to-document recheck ran ordinary owner tests at the reviewed HEAD
with the dirty documentation/release-tooling overlay. Crate source files were
unchanged. Rust used toolchain 1.99.0, `cargo test --locked`, four build jobs and
four test threads; these are focused debug-profile results:

| Scope | Executed result |
|---|---|
| Contract: builders, topology, plan validation and snapshot oracle | 54 passed |
| Engine: library plus identity, substrate, effect retirement, memory epochs, pending resolver, fairness and queue history | 109 passed |
| Host: default library/physical authority; Rayon library, dispatch, authority/protocol, deadline custody, drain, accounting fault and strict ingress | 132 passed (30 default + 102 Rayon; feature variants include repeated unit cases) |
| Benchmark structural oracles: `host_composite`, `fairness_property` | 17 passed; six 500-seed populations retain all 3000 fairness seeds |
| Python: PM, release, ticket/scenario maps, batch custody, generated-denominator validator and model-producer tests | 316 passed; no mutation campaign executed |

Architecture/map validation, prompt check, gate inventory, affected Python Ruff
and Semgrep passed. The 101 deleted-input dispositions match their original Git
bytes and successors; all six Sep-16/Sep-22 raw JSON artifacts remain byte-identical.
These owner results do not replace final clean-source CI, full23, human release
adjudication or consumer acceptance. Current canonical local artifact observations
are recorded in the remaining-work list with their historical source identities.

[Remaining work](../remaining-work.md) separates final-source proof, human
compatibility, measured performance and consumer acceptance. The source-bound
policy remains ADR 0006; release procedure remains the release checklist.
