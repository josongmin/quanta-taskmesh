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
6. deterministic reduce enforcement for fan-out stages
7. snapshot + substrate inventory SSOT
