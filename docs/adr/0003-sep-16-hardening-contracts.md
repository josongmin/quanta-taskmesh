# 0003. Hardening contracts (D01–D17)

- Status: Accepted
- Date: 2026-09-16; consolidated 2026-10-07
- Decision owner: Song Min
- Predecessors: [0001](0001-hexagonal-feature-sliced-architecture.md),
  [0002](0002-drr-proportional-fairness.md)
- Amendments: [0004](0004-sep-25-ingress-plan-identity-and-wire.md),
  [0005](0005-sep-25-execution-response-and-custody.md),
  [0006](0006-source-bound-verification-authority.md),
  [0009](0009-runtime-and-evidence-hardening.md)

## Context

Admission, physical capacity, memory accounting and worker lifetime require one
owner for each transition. These decisions consolidate the implemented Sep-16
contracts and their Sep-21 amendments. D01–D17 remain stable reference IDs.
Historical test counts and obsolete API descriptions are archived by Git source
and byte digest in [document history](../evidence/document-history.json).
[0008](0008-audit-implementation-record.md) retains finding/ticket disposition;
this ADR does not approve compatibility or qualify a later source revision.

## D01 — Atomic bounded admission

The first admission transition reserves semantic capacity and physical capability
together, enters the class's bounded queue, or rejects. A semaphore or executor
queue before admission cannot supply an unbounded competing wait path.
`OverflowPolicy::Reject` rejects physical saturation; queueing requires explicit
positive depth. Legacy role-slot `0` skips that role gate, while CPU and physical
worker limits remain finite. `CpuMode::Fixed(0)` and
`PhysicalDomainMode::Fixed(0)` reject with typed errors.
Owner: [Governor](../../crates/taskmesh-engine/src/engine/governor.rs),
[topology](../../crates/taskmesh-contract/src/topology.rs).

## D02 — Reservation, measurement and release

The ledger separates immutable `original_estimate_units`, decreasing
`remaining_reservation_units`, and charged `effective_units`. Estimated accounting
uses remaining reservation; reconcile cannot reacquire released units.
`MemoryReleasePolicy::OnTaskCompletion` refuses stage release; eligible policies
return a typed `StageReleaseOutcome`. Measurement and release sequences reject
stale updates. Implicit reconcile returns `ReconcileOutcome`; `u64::MAX` exhaustion
returns `EpochExhausted` without changing held memory or activity time.
Owner: [memory governance](../../crates/taskmesh-engine/src/engine/governor/memory_governance.rs).

## D03 — One-hop resource fallback

`DegradeToLight` changes resource accounting class while retaining declared
cancellation, deadline and capability requirements. A fallback that degrades
again, cycles, or is disabled rejects during policy validation. This prevents
configuration from promising an unimplemented second hop. The declared class
remains in `TaskSpec`; the effective class is in the permit ledger.
Owner: [policy validation](../../crates/taskmesh-engine/src/engine/governor/validation.rs).

## D04 — Requested stack consumes its real capability

A blocking-family submission with `stack_size_bytes` reserves `large_stack`
capability and the dedicated physical domain regardless of the hint. A hint
cannot bypass the actual OS-thread substrate limit. Stack size is validated
before spawning; platform overflow/panic is not an accepted validation method.
Owner: [host runtime](../../crates/taskmesh/src/runtime.rs).

## D05 — Validate and freeze the executor descriptor

`Builder::build` rejects inline submission, unknown worker count/domain, or a
worker count unequal to the resolved physical domain. CPU adapters declare
`physical.shared_blocking` or `physical.cpu`. One validated descriptor is frozen
for preflight, dispatch and topology; dispatch does not query the adapter again.
The legacy trait default preserves compilation, not installation acceptance.

The descriptor is a trusted adapter declaration. Taskmesh bounds its submissions;
ambient Tokio/Rayon users and runtimes without a shared Governor are outside that
bound. `exclusive_pool` distinguishes that scope. Semantic class policy does not
create engine-specific worker pools or another host queue.
Owner: [Builder](../../crates/taskmesh/src/builder.rs),
[executor ports](../../crates/taskmesh-contract/src/ports.rs).

## D06 — Checked aggregate and exact wire accounting

Request cost is `u32`; aggregate resource accounting uses checked `u128`.
Snapshot held amounts use decimal strings to preserve exact JSON round trips.
Bytes-to-units conversion is fallible. Overflow cannot saturate into apparent
capacity: sticky `AccountingFault` rejects new work and terminalizes/wakes queued
work while live worker leases remain charged until settlement.
Owner: [snapshot](../../crates/taskmesh-contract/src/snapshot.rs),
[state transitions](../../crates/taskmesh-engine/src/engine/state/transition.rs).

## D07 — Monotonic commit time without callbacks under lock

Sample `Clock` outside the state mutex, then commit `max(sample, watermark)` and
advance the watermark inside the transition. This prevents stale activity time
without allowing callback reentrancy under the lock. It is logical commit time,
not a measured lock-wait latency bound. Waker calls and retired-value drops also
occur outside the state lock.
Owner: [engine shared effects](../../crates/taskmesh-engine/src/shared/mod.rs).

## D08 — Fairness selects runnable work

Candidates satisfy semantic and physical limits. Same-class FIFO permits a
newcomer to bypass a head blocked on another capability pool; a queued successor
still cannot pass its own head. WFQ uses positive weights and nonzero fixed-point
increments, rebuilds unserved debt after cancellation/bypass, and preserves
canonical tail tags on head service. Idle queues do not retain credit. DRR uses
positive quantum and arithmetic ring traversal bounded by class count.

Promotion checks its budget before state-mutating selection. FIFO/WFQ/deadline
ties use lock-committed physical `queue_order`, not preallocated handle order.
During a continuation gap, a queue-capable newcomer with an empty own queue joins
the queue for tier selection. A runnable same-class head retains precedence; a
head blocked on another pool retains the capability exception. A nonqueueing
class has no waiting space and may admit. Intake block reason is diagnostic
metadata, not a dynamically updated scheduling input. These choices bound queue
state while accepting pool-specific head-of-line loss for already queued work.
Owner: [scheduler](../../crates/taskmesh-engine/src/features/fairness/scheduler.rs),
[pending admission](../../crates/taskmesh-engine/src/features/admission/pending.rs).

## D09 — One acquisition handoff arbiter

Immediate and queued paths use cancel → absolute deadline → relative timeout
precedence. Recheck before handing a permit to work; an expired permit is returned
unstarted. Relative ZERO is try-once and does not override an expired absolute
deadline. Equality expires. A last engine lookup may preserve a more specific
terminal reason, but cannot authorize a late start. This is a post-wait check,
not a hard response-latency bound under OS preemption.
Owner: [acquisition](../../crates/taskmesh/src/runtime/acquisition.rs).

## D10 — Caller response and worker custody

Cancellation, deadline and caller drop end the caller's wait. A started worker
keeps its execution lease through actual termination and owned-runtime teardown.
Success becomes visible only after the release fence and terminal recheck.
`RunFor` starts at worker start; blocking work is not abortable merely because the
caller received a deadline response. Requested-stack panic/deadline/cancel can be
reported while teardown continues under custody. Synchronous `CompleteBy` is
refused; explicit `*_response_by` APIs own their separate caller-response budget.
Owner: [detached execution](../../crates/taskmesh/src/runtime/detached.rs),
[synchronous APIs](../../crates/taskmesh/src/runtime/synchronous.rs), ADR 0005.

## D11 — Validate, render, then apply prompt routing

PM check is read-only. Build validates all registries, relative-path identities,
unique YAML keys, references and budgets before writing marked router blocks.
It does not overwrite arbitrary target documents or provide a multi-file atomic
transaction. Contract text stays in its owning ADR/spec/tool file.
Owner: [PM policy](../../tools/pm/README.md).

## D12 — Reject declared wait cycles; retain caller-owned payloads

Child builders require root, immediate parent operation and parent stage. Missing
parent identity rejects rather than being inferred. Validated plans freeze their
identity; live parent permit generations prevent reuse of an operation name from
reattaching an old lineage. Admission and promotion inspect all declared blockers
and continuous awaited-ancestor links before returning `NestedWaitCycle`.
Sibling or stranger capacity that can independently release does not prove a
cycle. Unknown waits inside opaque closures and undeclared cross-root graphs are
not inferred. Plan membership is a caller contract, not an implicit DAG executor.
Borrowed/`!Send` payloads remain in the caller task; the kernel owns metadata.
Owner: [task scope](../../crates/taskmesh-contract/src/task.rs),
[execution plan](../../crates/taskmesh/src/execution_plan.rs), ADR 0004–0005.

## D13 — Model-check production governance

The synchronization seam compiles the production Governor against the selected
checker's mutex/atomics. Loom or Shuttle cfg and feature must be enabled together;
half-configured builds reject instead of silently running zero tests. Handwritten
replicas cannot establish production behavior. Model files or historical schedule
counts do not establish execution at the current source.
Owner: [sync seam](../../crates/taskmesh-engine/src/sync.rs), ADR 0006.

## D14 — Typed failures and protected lease authority

Unknown classes, foreign Governor handles, invalid identities and unsupported
deadlines reject. Identity counters do not wrap. Outcome types are `must_use`;
worker panic, unavailability and abandoned jobs retain distinct typed failures.
The first advance out of `DispatchReserved` mints one non-clone `LeaseToken`.
Afterward `release`/`abandon` cannot reclaim worker-owned capacity;
`release_leased(token)` owns settlement. A host lacking that token cannot start
work. Dropping a token does not prove settlement or permit automatic refund.
Adapters, embedder access and custom Clock ports are trusted; these guards prevent
ownership mistakes and do not form a security sandbox.
Owner: [Governor](../../crates/taskmesh-engine/src/engine/governor.rs), ADR 0004–0005.

## D15 — Activity and epoch assignment belong to the transition

Claim touches the lease. Implicit reconcile assigns and applies its epoch in the
same lock transition. Terminal ticket retention has a bounded indexed policy;
faulted queued tickets remain held until claim/abandon or Governor destruction.
Pure spec precheck stays outside the lock. Root identity must distinguish actual
root executions; operation-name reuse cannot manufacture authority.
Owner: [state transitions](../../crates/taskmesh-engine/src/engine/state/transition.rs).

## D16 — Source-bound proof and self-tested producers

The gate inventory owns actual discovered Rust/Python/fuzz targets and required
commands. Architecture checks use the Cargo dependency graph; Semgrep rules need
positive/negative fixtures. Mutation evidence requires a complete raw identity
set, named rejection oracle, producer completion and exact source/tool identity.
Partial or cached diagnostics cannot become final clean-source qualification.

IAI baseline continuity binds its compiler/runner/source fingerprint; initial
baseline creation is not regression PASS. Allocation and instruction thresholds
remain in [perf-gate.json](../../tools/bench/perf-gate.json), not copied test counts.
The allocation counter self-check observes 1,000 known allocations. CI, nightly,
release adjudication, performance and consumer acceptance have separate verdicts.
Mutation/final release campaigns require explicit current-request authorization
under AGENTS.md. Ordinary owner checks and CI remain distinct from those campaigns.
Owner: [ADR 0006](0006-source-bound-verification-authority.md), gate inventory.

## D17 — Engine closes admission; host waits for custody

`close_admission` is one-way and linearized with admission under the engine lock.
Later valid submissions reject `RuntimeUnavailable` without queueing/accounting;
preflight can still return its earlier typed error. Already accepted/queued work
continues promotion, claim and settlement. Host drain waits for all class inflight
and queued gauges to reach zero. Timeout returns the outstanding map and leaves
the runtime draining; a later call resumes waiting. Shared Governor custody is
included, even for direct embedder admission. Host-owned settlement wakes drain;
external releases may only become visible at the timeout's final observation.
Drain does not promise forced termination of started blocking work.
Owner: [drain](../../crates/taskmesh/src/runtime/drain.rs).

## Consequences

Finite queues, explicit topology and typed ownership failures increase validation
and bookkeeping cost. Public API/wire migration is owned by the current specs,
CHANGELOG and ADR 0009 compatibility input. D01–D17 are implemented contracts;
current-source proof, human release decisions and activation remain in
[remaining work](../remaining-work.md).
