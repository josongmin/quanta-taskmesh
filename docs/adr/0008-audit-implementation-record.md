# 0008. Historical audit implementation and evidence disposition

- Status: Accepted as repository implementation history; no release approval
- Date: 2026-10-07
- Consolidation source: `2123a462764a3d6445fd536a1e5a5f99d019425e`
- Replaces: Sep-16 finding tickets and Sep-21 tickets/execution packets. Original
  paths and byte digests are recorded in [document history](../evidence/document-history.json).

## Decision

Keep established contracts in ADR 0003–0006, later structural repairs in
[0009](0009-runtime-and-evidence-hardening.md), and only unfinished work in
[remaining work](../remaining-work.md). Historical ticket lifecycle values retain
their original scope. `LOCALLY_VERIFIED` is historical owner evidence, not a
current clean-source or release verdict. V01/V02/R01 remain unqualified.
Current release qualification instead requires the exact-source ordinary full23
receipt and 23 finding witnesses; it does not rewrite those historical states.

The schema-2 [Sep-21 map](../evidence/sep21/ticket-map.json) retains all 12 tickets,
23 findings and each ticket's original acceptance IDs. Release tools read each exact section below and
bind its ADR file digest. Old receipts/manifests require regeneration against the
new tracked map and source; no historical hash or verdict is rewritten.

## Sep-16 disposition

The 40 TM16 observations at `9ae9547216c70458f54f37368a67661321060886`
are historical inputs, not today's defect queue. Their contract decisions and
subsequent fixes are retained below; the old defect-asserting standalone harnesses
are recoverable from Git, and are not current regression suites. TM16-005 was a
contract decision. Linux measurement, release and consumer acceptance remain
separate work. Original reproduction greens never proved remediation.

| Finding | Implemented boundary / durable authority |
|---|---|
| TM16-001 | Atomic bounded semantic/physical admission; 0003 D01, 0005. |
| TM16-002 | Worker-start RunFor response budget; 0003 D10, 0005. |
| TM16-003 | Fallible topology/build validation; 0003 D14. |
| TM16-004 | Explicit inventory authority; 0003 D14, 0004. |
| TM16-005 | Stage release policy and distinct ledger facts; 0003 D02. |
| TM16-006 | Inventory-backed enforced gate graph; 0006. |
| TM16-007 | Dependency policy remains in config/deny.toml and Cargo.lock; new advisories need current checks. |
| TM16-008 | Checked resource conservation and fail-stop accounting; 0003 D06, 0009. |
| TM16-009 | Applied stage activity updates lease time; 0003 D07/D15. |
| TM16-010 | Reclaimed tickets cannot authorize dead permits; 0003 D14. |
| TM16-011 | Reconcile charges remaining reservation; 0003 D02. |
| TM16-012 | Arithmetic bounded DRR selection; 0002, 0003 D08. |
| TM16-013 | Nonzero scale-invariant WFQ increments; 0003 D08. |
| TM16-014 | Cancellation removes unserved WFQ debt; 0003 D08. |
| TM16-015 | Success after release fence; 0003 D10, 0005. |
| TM16-016 | Complete negative-tested crate/dependency boundary inventory; 0001, 0003 D16. |
| TM16-017 | Actual test discovery and Semgrep enrollment; 0006. |
| TM16-018 | Benchmark operations require admitted work; 0003 D16, 9000. |
| TM16-019 | Pipeline/status/lookup failures reject evidence; 0006, 9000. |
| TM16-020 | IAI compatible fingerprint, threshold and baseline contract; 0003 D16, 9000. New Linux execution remains required. |
| TM16-021 | Consumer MSRV separate from developer/proof toolchain; 0006. |
| TM16-022 | Relative run budget starts at worker start; 0003 D10, 0005. |
| TM16-023 | Requested-stack consumes actual dedicated domain; 0003 D04/D05, 0005. |
| TM16-024 | Caller response can precede teardown; capacity survives until settlement; 0005. |
| TM16-025 | PM registry/router single authority; 0003 D11; tools/pm. |
| TM16-026 | Monotonic commit-time activity; 0003 D07/D15. |
| TM16-027 | Trace timing/order validation; 9000. |
| TM16-028 | MMPP phase boundaries and positive dwell validation; 9000. |
| TM16-029 | Open-loop population is not omission-corrected twice; 9000. |
| TM16-030 | Waker retirement/effects outside governor lock; 0003 D07, 0009. |
| TM16-031 | Fallible thread setup and safe labels; 0003 D14. |
| TM16-032 | Acquisition expiry rechecked at handoff; 0003 D09, 0005. |
| TM16-033 | Actual gate inventory replaces phantom commands; 0006. |
| TM16-034 | IAI op-only/input teardown measurement unit; 0003 D16, 9000. |
| TM16-035 | PM full template identity; 0003 D11/D16. |
| TM16-036 | Criterion completed-operation denominator; 9000. |
| TM16-037 | Empty DRR class cannot bank credit; 0002, 0003 D08. |
| TM16-038 | Malformed allocation metric rejects; 0003 D16. |
| TM16-039 | Duplicate PM target keys reject; 0003 D11/D16. |
| TM16-040 | Mixed soak asserts real workload outcomes and accounting; 0003 D16. |

Historical review limits are preserved: WFQ `burst` is reserved metadata,
`DropBestEffort` rejects rather than evicting, checkpoint/stage/reduce declarations
are caller-owned, measured memory is accounting rather than an allocation hard
cap, and a per-runtime pool bound is not a global bound over unrelated callers.
The Sep-16 exception register's inactive retirement worker/diagnostic ring limits
remain N/A unless those components are introduced. Its Linux IAI, human rollback
approval and consumer adoption exceptions survive in the remaining-work list.
Speculative quality candidates without a current counterexample are not promoted
to active defects.

## SEP21-C01 — Validated plan and neutral provenance

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-010
- Decision authority: ADR 0003, 0004; owner `crates/taskmesh-contract/tests/task_plan_validation.rs`.

One plan validator rejects duplicate/conflicting stages and invalid lineage. `PlanSource` is a bounded opaque string; legacy valid JSON strings survive while Rust enum/Copy compatibility changes. Exact parent operation is required.

### Closure evidence

Original ticket at the consolidation source records 7 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-E01 — Capability authority

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-008
- Decision authority: ADR 0003, 0004; owner `crates/taskmesh-engine/tests/identity_authority.rs`.

Capability handles are opaque and inventory-bound; fabricated or foreign handles reject before admission.

### Closure evidence

Original ticket at the consolidation source records 5 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-E02 — Effect custody

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-004
- Decision authority: ADR 0003, 0005, 0009; owner `crates/taskmesh-engine/src/features`.

State commits and effect delivery are separated. Waker/drop callbacks execute outside the lock; failed delivery compensates only untransferred ownership, and a worker lease cannot be abandoned into free capacity.

### Closure evidence

Original ticket at the consolidation source records 5 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-E03 — Memory measurement sequence

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-016
- Decision authority: ADR 0003; owner `crates/taskmesh-engine/src/features/memory`.

Implicit and explicit measurement sequence exhaustion returns a typed refusal without changing held charge or activity; release still settles the permit.

### Closure evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-E04 — Pending resolver and lineage

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-001, TM21-002, TM21-009, TM21-023
- Decision authority: ADR 0003, 0004; owner `crates/taskmesh-engine/src/features/composite`.

Resolve live exact-parent generations and declared wait chains across all blockers. Active identity conflicts reject; promotion rechecks cycles without treating releasable siblings/strangers as ancestors.

### Closure evidence

Original ticket at the consolidation source records 8 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-H01 — Validated dispatch plan

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-007
- Decision authority: ADR 0004, 0005; owner `crates/taskmesh/src/execution_plan.rs`.

Resolve stage/stack/capability requirements once before admission; dispatch consumes that validated plan. One host call executes its first declared stage.

### Closure evidence

Original ticket at the consolidation source records 5 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-H02 — Acquisition arbiter

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-005, TM21-006, TM21-019
- Decision authority: ADR 0005; owner `crates/taskmesh/src/runtime`.

Immediate and queued handoff use cancellation, absolute deadline, then relative timeout precedence. ZERO is try-once. Refusal unwinds unstarted ownership and cannot free a started worker lease.

### Closure evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-H03 — Executor and physical domain

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-003, TM21-015, TM21-020
- Decision authority: ADR 0003, 0005; owner `crates/taskmesh-contract/src`.

Freeze a validated executor descriptor; reserve semantic and physical capacity atomically. Shared blocking, CPU and dedicated domains remain bounded without a second host queue or engine-specific pools.

### Closure evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-V01 — Trusted producer envelopes

- Historical status: IMPLEMENTED_UNQUALIFIED
- Findings: TM21-011, TM21-014, TM21-022
- Decision authority: ADR 0006, 0009; owner `tools/qualification`.

Tracked producer contracts bind structured raw, environment, source and execution custody. Summary verdicts cannot replace raw validation, a clean source receipt or an independent adjudication.

### Producer evidence

Original ticket at the consolidation source records 7 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.
Historical producer/checkpoint results are partial or source-specific. Required
final-source execution and/or human release adjudication remain open.

## SEP21-V02 — Mutation denominator

- Historical status: IMPLEMENTED_UNQUALIFIED
- Findings: TM21-012, TM21-017
- Decision authority: ADR 0006; owner `tools/verification/run_generated_mutants.py`.

Complete planned/categorized/executed identity sets and raw outcomes are mandatory. Compile-unviable limitations remain explicit; partial, missed, interrupted or stale campaigns cannot qualify.

### Producer evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.
Historical producer/checkpoint results are partial or source-specific. Required
final-source execution and/or human release adjudication remain open.

## SEP21-V03 — Search and model evidence

- Historical status: LOCALLY_VERIFIED
- Findings: TM21-018, TM21-021
- Decision authority: ADR 0003, 0006; owner `tools/modelcheck`.

Target discovery, semantic checkpoints and recorded search budgets drive production-Governor models and fuzz evidence; missing targets/bounds/counts cannot pass. Model presence is not an executed verdict.

### Closure evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.

## SEP21-R01 — Release compatibility and qualification

- Historical status: IMPLEMENTED_UNQUALIFIED
- Findings: TM21-013
- Decision authority: ADR 0006; owner `tools/release`.

Release collection recomputes exact-source ordinary and finding evidence, four-crate semver outputs, human Rust/wire/behavior dispositions and version approval. Its implementation does not approve a release.

### Producer evidence

Original ticket at the consolidation source records 6 acceptance IDs.
The map preserves their identity; Git retains exact test/receipt narratives.
Historical producer/checkpoint results are partial or source-specific. Required
final-source execution and/or human release adjudication remain open.
