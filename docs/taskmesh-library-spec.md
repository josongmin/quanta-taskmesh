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
     `MalformedTask`, `SubstrateMismatch`, `SubstrateSaturated`,
     `SubstratePoolTimedOut`)
4. `Snapshot` (`SNAPSHOT_SCHEMA_VERSION = 2`)
   - per-class inflight/queued + exact (`u128`, decimal-string wire) held units
   - ownership-phase gauges (`dispatch_reserved`/`accepted`/`running`/
     `cleanup_pending`) that partition `inflight`, plus cumulative
     `admitted_total`/`started_total`/`terminated_total`
   - capability-pool occupancy `(in_use, limit)` and the substrate inventory
   - `conservation_violation()` checks the identities in one snapshot
5. `TerminalReason` / `GovernorError::TicketClaimTerminated` — a queued request
   that ended before its waiter claimed it says so, with the reason

## Runtime Highlights

1. `run_io`     — async, `Send`
2. `run_blocking` — blocking pool; join failure is governor-side
3. `run_cpu`    — pluggable `CpuExecutor` (blocking-pool default, or Rayon)
4. `run_local`  — non-`Send`, local-runtime exception only (guarded)
5. `run_async_with_requested_stack` — `!Send` async root on an owned
   current-thread Tokio runtime hosted by the requested-stack worker

The `*_with` variants accept `SubmitOptions` for pre-submit cancel, mid-run
cooperative cancel/run-deadline, one absolute admission-to-completion deadline,
and a bounded acquire wait. The acquire budget covers *every* wait on the way
in — the admission lock included — so an `Admitted` that arrives after the
budget expired is unwound, not started. `Duration::ZERO` keeps its
"try, do not wait" meaning.

## Engine Highlights

1. fail-closed class lookup (unknown/disabled reject, never default-admit)
2. bounded per-class queues; cross-class fairness (FIFO/WFQ/DRR/EDF/scavenger).
   Candidates must satisfy semantic capacity **and** the capability pool their
   head froze at intake; there is no second scheduling queue behind the first.
   DRR walks its ring arithmetically (`O(classes)`, never `O(cost/quantum)`) and
   resets credit when a queue drains; WFQ keeps proportional share across the
   whole accepted weight range and drops no service debt for cancelled work
3. permit admission/release inseparable from inflight + resource accounting.
   Capability-pool occupancy is decided in the same transition — the engine is
   the single authority for physical *and* semantic capacity. Aggregates are
   exact (`u128`, checked comparisons); nothing saturates
4. memory governance: estimated/measured/hybrid reconcile, overcommit, leak
   sweep. The ledger keeps `original_estimate` / `remaining_reservation` /
   `effective` apart: a reconcile never resurrects units a stage boundary
   returned; `MemoryReleasePolicy` is enforced (`OnTaskCompletion` refuses a
   stage release with `StageReleaseOutcome::PolicyForbids`); measurement epochs
   and stage sequences reject stale/duplicate reports; the sweep reclaims only
   permits that never reached an executor (stale is not dead)
4b. lock discipline: the driven `Clock` is sampled outside the state mutex and
   clamped to a monotonic commit watermark inside it; host `Arc`s (wakers) are
   woken **and dropped** outside the lock; promotion is bounded per pass
   (`PROMOTION_BUDGET`) with guaranteed continuation
4c. one ticket lifecycle: `claim` returns `ClaimOutcome::{Ready, Pending,
   Terminal(reason), Invalid}` — a reclaimed reservation is a terminal answer,
   never a dead permit id and never an endless wait; terminal retention is
   bounded (`MAX_TERMINAL_TICKETS`) and `O(log n)` to observe. `claim` is lease
   activity (the sweep's staleness clock restarts at the claim). `release`
   returns `ReleaseOutcome` (`#[must_use]`): `UnknownPermit` is a double release
   or a reclaimed lease, never a silent no-op
4d. in-class FIFO across the promotion budget: a pass grants at most
   `PROMOTION_BUDGET`, checks the budget *before* selecting (so no class is
   charged for a dispatch that is not made), and a newcomer that fits while an
   earlier runnable head of its class is still queued is placed behind it
   (`CapacityBlock::QueuedBehind`); the only overtake is a head blocked on a
   different capability pool
4f. typed refusals instead of silent drops: `SubmitOptions::deadline` on a class
   whose `CancellationPolicy` is not `CooperativeWithDeadline` →
   `GovernorError::DeadlineUnsupported { class, policy }`; worker failures are
   `WorkerPanicked` / `WorkerUnavailable` (never started) / `JobAbandoned`; a
   lease the leak sweep reclaimed before dispatch → `LeaseReclaimed`
4g. `CpuExecutor::capabilities()` is load-bearing: `Builder::build` refuses an
   adapter whose `declared_workers` is below the resolved `cpu` gate
   (`TopologyError::ExecutorDeclaresFewerWorkers`); `TokioRuntime::executor_capabilities()`
   exposes the declaration (Rayon: real thread count, exclusive unless
   `with_pool`; Tokio blocking pool: workers unknown, shared)
4e. concurrency proofs run the production `Governor`: under `--cfg loom
   --features loom` / `--cfg shuttle --features shuttle` the engine's `sync`
   seam swaps its mutex and atomics for the checker's, and
   `tests/loom_governance.rs` (exhaustive) / `tests/shuttle_governance.rs`
   (randomized) drive `admit`/`claim`/`abandon`/`release`/`reap_leaks`/
   `reconcile_memory` directly
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

## Admission verdicts

One intake transition checks the limits in this order and reports the first
one that blocks; a queueing class then queues within `max_queue_depth` and a
non-queueing class is shed with the verdict of that limit. A *full* queue sheds
with the verdict of the limit its head is blocked on, not with `QueueFull`,
unless that limit is the class quota or the cpu budget.

| Limit (checked in this order) | `CapacityBlock` | Shed verdict (no queue / queue full) |
|---|---|---|
| unknown / disabled class, recursive child, malformed plan | — | `UnknownClass` / `ClassDisabled` / `RecursiveAdmission` / `MalformedTask` |
| class `max_inflight` reached | `Inflight` | `CpuSaturated` (no queue) / `QueueFull` (queue full) |
| resolved capability pool at its limit | `Capability` | `SubstrateSaturated` |
| global cpu budget | `Cpu` | `CpuSaturated` / `QueueFull` |
| global memory budget | `Memory` | per `MemoryOvercommitPolicy`: `Queue` queues (full → `MemorySaturated`), `Reject` → `MemorySaturated`, `DegradeToLight` re-accounts once under the fallback class |
| capacity free but a promotion pass is owed (D08 gap) | `QueuedBehind` | never shed here: the class demonstrably has a queue and the request joins it |
| ledger cannot represent the total | `AccountingFault` | `RuntimeUnavailable` (sticky) |

`CpuSaturated` is the class-busy verdict even for a class whose `cpu_units` is
`0` (the name is historical; the cause is the class quota). The block reason a
queued ticket reports (`Governor::pending_block_reason`) is the one recorded at
intake and is diagnostic only.

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
  `maintenance_workers`, and the topology-sized CPU pool); `0` = unlimited.
  They are validated before anything is sized (`TopologyConfig::validate`;
  an inverted worker window or an unrepresentable slot count is a typed
  `GovernorError::InvalidTopology`, not a panic), and detected parallelism is
  read once per build and shared. Occupancy is part of the admission decision:
  a full pool sheds a non-queueing class immediately as `SubstrateSaturated`,
  and a queueing class waits within its own `max_queue_depth` and reports
  `SubstratePoolTimedOut` on expiry (distinct from the governor-queue
  `PermitAcquireTimedOut`). A stack request consumes the `large_stack` pool
  whichever blocking-family hint carried it. `Blocking`/`LargeStack`/`Background`
  share the blocking executor but hold separate capability pools.
- **cancellation_policy** gates mid-run cooperative cancel across
  `run_io`/`run_local`/`run_cpu`/`run_blocking` → `GovernorError::Cancelled` (on the
  synchronous paths the caller's wait ends; a started job runs on, charged); `run_cpu` escapes
  even a stalled `CpuExecutor`, while its queued or running worker closure keeps
  the execution lease until that closure runs or is dropped.
  `CooperativeWithDeadline` additionally honors `SubmissionDeadline::RunFor`
  for execution-only budgets and
  `SubmissionDeadline::CompleteBy` for one substrate/admission/execution bound;
  both map expiry to `GovernorError::DeadlineExceeded`. `RunFor` is anchored
  to the **worker's own start timestamp** on every detached path, so an
  executor that runs the job inline before `spawn` returns cannot report a
  late result as `Ok`; a completion at or past the deadline is a miss.
  On `run_blocking` a `RunFor` bounds the *caller's wait*: a started blocking
  job cannot be aborted, and the contract does not pretend otherwise.
  `CompleteBy` is admitted only on cooperative async paths and is polled by the
  runtime that owns the work; blocking and CPU work reject it before invoking
  the job. A non-yielding poll retains its permit until it yields, so governed
  counts never understate live work.
- **response is not custody.** A deadline reply, a cancel, or a dropped caller
  future ends the caller's wait only. `run_blocking`, requested-stack blocking
  and async, and `run_cpu` retain the execution lease through actual worker
  termination — including an owned runtime's teardown, which cannot abort a
  started blocking child — and the snapshot shows the work as `running` /
  `cleanup_pending` until then. On normal completion custody travels with the
  result, so a caller that can observe the value observes released capacity.
- **worker setup failures are typed.** The OS thread label for a dedicated
  worker is derived from the class name, sanitized and bounded; a class
  containing a NUL or exotic characters runs, it does not panic the caller.
  Task error, worker panic, spawn failure, and a receiver that stopped waiting
  are distinct outcomes; the last is expected and hands the release to the
  worker.
- **requested-stack async execution** requires both
  `SubstrateHint::LargeStackCapability` and a requested stack size. Missing or
  invalid shape, worker spawn/runtime initialization failure, and worker panic
  are fail-closed governor errors. Its admission permit and capability slot stay
  held until the owned root completes, is cancelled, or its caller drops.
- **memory_release_policy** is enforced: `OnStageBoundary` and `LeakDetecting`
  permit `release_stage_memory`; `OnTaskCompletion` reports
  `StageReleaseOutcome::PolicyForbids`. `LeakDetecting` is the opt-in for
  leak-sweep reclaim, and the sweep reclaims only permits that never reached
  an executor (`retained_active` reports the rest). A downward
  `reconcile_memory` frees budget and promotes queued work.
- **supported nesting is declared, not inferred.** A parent that awaits a child
  on the same capability it holds can deadlock a bounded pool; bounded queues
  are not deadlock avoidance, and the runtime does not inspect opaque closures
  for wait graphs. Declared dependency cycles are out of scope (ADR 0003, D12).
- `checkpoint_policy` is host-inspected metadata (engine preserves, does not
  enforce).
