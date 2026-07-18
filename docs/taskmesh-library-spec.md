# Taskmesh Library Spec

## Package Split (hexagonal)

1. `taskmesh-contract` — frozen public vocabulary **and** port surface
   (`Runtime`, `CpuExecutor`, `Clock`, `PermitWaker`). Pure data + traits.
2. `taskmesh-engine` — governance core (admission, fairness, memory, composite,
   inventory). Pure, runtime-agnostic. Inside the hexagon.
3. `taskmesh` — Tokio host facade. Implements `Runtime`, owns the default CPU
   executor (blocking pool), cancel/deadline adapter, and the builder.
4. `taskmesh-rayon` — optional shared-CPU executor adapter (one `CpuExecutor`).

Dependency direction always points inward: adapters depend on the contract and
engine; the engine depends only on the contract; the contract depends on nothing
but `serde`.

## Contract Highlights

1. `TaskSpec`
   - root operation id, stage descriptors (with `fan_out` + reduce policy)
   - product-neutral source/reason
2. `ClassPolicy`
   - fairness, memory mode/release/overcommit, overflow, retry-after, checkpoint
3. `AdmissionVerdict`
   - typed rejection surface (incl. `RecursiveAdmission`, `PermitAcquireTimedOut`,
     `MalformedTask`, `SubstrateMismatch`, `SubstratePoolTimedOut`)
4. `Snapshot`
   - per-class inflight/queued/held + substrate inventory

## Runtime Highlights

1. `run_io`     — async, `Send`
2. `run_blocking` — blocking pool; join failure is governor-side
3. `run_cpu`    — pluggable `CpuExecutor` (blocking-pool default, or Rayon)
4. `run_local`  — non-`Send`, local-runtime exception only (guarded)
5. `run_async_with_requested_stack` — `!Send` async root on an owned
   current-thread Tokio runtime hosted by the requested-stack worker

The `*_with` variants accept `SubmitOptions` for pre-submit cancel, mid-run
cooperative cancel/run-deadline, and a bounded acquire wait (covering both the
governor queue and the substrate capability-pool slot).

## Engine Highlights

1. fail-closed class lookup (unknown/disabled reject, never default-admit)
2. bounded per-class queues; cross-class fairness (FIFO/WFQ/DRR/EDF/scavenger)
3. permit admission/release inseparable from inflight + resource accounting
4. memory governance: estimated/measured/hybrid reconcile, overcommit, leak sweep
5. composite root attribution + recursive-admission guard (keyed on the child's
   authoritative declared `parent_stage`, not its substrate)
6. deterministic reduce enforcement for fan-out stages (rejected at admission)
7. snapshot + substrate inventory SSOT (built-ins seeded at `PolicySet::new`, so
   every governor — direct or host-built — has an authoritative inventory)
8. fairness homogeneity validated per tier at construction (no silent override)
9. adaptive retry-after blends contention with the class fairness params
10. classification provenance (source + reason) preserved intact through
    queue→promote→claim, queryable per permit (`Governor::permit_provenance`)
11. admission key is derived authoritatively from the root operation id — no
    caller-supplied key (the old key carried no behavior, so it was removed)
12. requested-stack async execution keeps the future factory, root, and Tokio
    children on the owned worker runtime instead of borrowing the caller runtime

## Enforcement (not advisory)

The host enforces the declared contract at runtime:

- **policy validity** is checked fail-closed at `Builder::build` /
  `Governor::new` (impossible budgets, mixed-tier fairness, invalid memory
  scaling, misused degrade/queue) — never deferred to a runtime admit path.
- **spec shape** is validated at admission: zero stages, an inconsistent
  per-stage class, or a fan-out stage missing its deterministic reduce policy
  all reject as `MalformedTask`.
- **substrate hint ↔ run path** must match (else `SubstrateMismatch`);
  `run_local` is the `LocalRuntime`-only exception.
- **topology slot counts** are real per-substrate capability-pool limits
  (`blocking_threads`, `large_stack_slots`, `local_runtime_slots`,
  `maintenance_workers`, and the topology-sized CPU pool); `0` = unlimited. A
  full pool backpressures as `SubstratePoolTimedOut` (distinct from the
  governor-queue `PermitAcquireTimedOut`). `Blocking`/`LargeStack`/`Background`
  share the blocking executor but hold separate capability pools.
- **cancellation_policy** gates mid-run cooperative cancel across
  `run_io`/`run_local`/`run_cpu` → `GovernorError::Cancelled`; `run_cpu` escapes
  even a stalled `CpuExecutor`. `CooperativeWithDeadline` additionally honors
  `SubmitOptions::deadline` → `GovernorError::DeadlineExceeded`.
- **requested-stack async execution** requires both
  `SubstrateHint::LargeStackCapability` and a requested stack size. Missing or
  invalid shape, worker spawn/runtime initialization failure, and worker panic
  are fail-closed governor errors. Its admission permit and capability slot stay
  held until the owned root completes, is cancelled, or its caller drops.
- **memory_release_policy::LeakDetecting** is the opt-in for leak-sweep reclaim;
  a downward `reconcile_memory` frees budget and promotes queued work.
- `checkpoint_policy` is host-inspected metadata (engine preserves, does not
  enforce).
