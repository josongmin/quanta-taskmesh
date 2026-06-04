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
   - typed rejection surface (incl. `RecursiveAdmission`, `PermitAcquireTimedOut`)
4. `Snapshot`
   - per-class inflight/queued/held + substrate inventory

## Runtime Highlights

1. `run_io`     — async, `Send`
2. `run_blocking` — blocking pool; join failure is governor-side
3. `run_cpu`    — pluggable `CpuExecutor` (blocking-pool default, or Rayon)
4. `run_local`  — non-`Send`, local-runtime exception only (guarded)

The `*_with` variants accept `SubmitOptions` for pre-submit cancel and a bounded
acquire wait.

## Engine Highlights

1. fail-closed class lookup (unknown/disabled reject, never default-admit)
2. bounded per-class queues; cross-class fairness (FIFO/WFQ/DRR/EDF/scavenger)
3. permit admission/release inseparable from inflight + resource accounting
4. memory governance: estimated/measured/hybrid reconcile, overcommit, leak sweep
5. composite root attribution + recursive-admission guard
6. deterministic reduce enforcement for fan-out stages (rejected at admission)
7. snapshot + substrate inventory SSOT
8. fairness homogeneity validated per tier at construction (no silent override)
9. adaptive retry-after blends contention with the class fairness params

## Enforcement (not advisory)

The host enforces the declared contract at runtime:

- **substrate hint ↔ run path** must match (else `MalformedTask`); `run_local` is
  the `LocalRuntime`-only exception.
- **topology slot counts** are real per-substrate capability-pool limits
  (`blocking_threads`, `large_stack_slots`, `local_runtime_slots`,
  `maintenance_workers`); `0` = unlimited.
- **cancellation_policy** gates mid-run cooperative cancel (async `run_io` only;
  sync blocking/cpu is pre-submit only) → `GovernorError::Cancelled`.
- **memory_release_policy::LeakDetecting** is the opt-in for leak-sweep reclaim.
- `checkpoint_policy` is host-inspected metadata (engine preserves, does not
  enforce).
