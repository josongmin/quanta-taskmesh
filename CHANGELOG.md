# Changelog

All notable changes to the taskmesh workspace (`taskmesh`, `taskmesh-contract`,
`taskmesh-engine`, `taskmesh-rayon`) are documented here. The four crates share
one workspace version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the major version is `0`, a minor bump (`0.1 → 0.2`) may contain breaking
changes; every one of them is listed under **Breaking changes and migration**.

The contract behind each entry is recorded in
[ADR 0003 — Sep-16 hardening contracts (D01–D17)](docs/adr/0003-sep-16-hardening-contracts.md);
entries cite the decision (`D..`) or audit finding (`TM16-..`) they implement.
The public surface a downstream consumer depends on is the `taskmesh` crate
(everyday SDK) and `taskmesh::ext` (direct engine embedding, custom adapters).

## [0.3.0] - Unreleased (NOT_QUALIFIED)

The workspace manifest and release policy select `0.3.0`; this heading does not
claim publication or qualification. The immutable compatibility baseline is the
0.2.0 release commit `39bee682d7daa1efaf1c10993ba6221fd0a90871`
(no tag). Final-SHA human API/wire/behavior adjudication, full generated mutation
quality, and a clean exact-source qualification and release receipt remain required.
The current operator path is local; hosted attestation remains a compatibility path. See
`tools/release/release-policy.json` and `docs/release-checklist.md`.

### Breaking changes and migration

- **Composite lineage (C01/E04).** The two-argument
  `child_of(root, parent_stage)` / `awaited_child_of(root, parent_stage)`
  builders are replaced with `(root, immediate_parent_operation, parent_stage)`.
  Before: `.awaited_child_of("root", TaskStage::new("stage"))`.
  After: `.awaited_child_of("root", "parent-operation", TaskStage::new("stage"))`.
  The child `TaskScope` JSON now requires `parent_operation_id`; old child
  payloads without it reject instead of guessing. Duplicate active
  `(root, operation)` and duplicate/conflicting stage descriptors reject
  before admission. A declared wait cycle follows exact parent identities
  through consecutive awaited ancestors, so capacity split across that chain
  rejects instead of queueing. A live parent permit generation is frozen at
  enqueue or grant, so reusing an operation name cannot rewrite ancestry. It is
  assessed against all blockers and re-assessed at promotion; unrelated
  siblings/strangers remain reversible.
- **Provenance API/wire (C01).** The `Copy` product enum `PlanSource` is now an
  opaque bounded, non-`Copy` value: use `PlanSource::new("consumer.key")?`
  or `PlanSource::INTERNAL`, not exhaustive product-variant matches. Legacy
  strings still deserialize and serialize unchanged during the 0.2 migration
  window; invalid/empty/oversize strings reject. This is a Rust source break,
  not a blanket wire break for valid legacy strings.
- **Capability authority (E01/H03).** Raw unknown/empty capability names no
  longer default to ungated admission. Resolve registry-issued handles under
  the same policy before admission; a foreign/unknown handle rejects without
  state change. Host dispatch freezes all physical and semantic capability
  requirements in one plan. `physical.shared_blocking`, `physical.cpu`, and
  `physical.dedicated` have finite limits; mixed blocking/CPU fallback work
  sharing an executor consumes one physical bound. Custom executors must
  declare nonblocking submit, physical domain, exact worker count, and whether
  submission requires an entered Tokio runtime. The built-in Tokio adapter
  declares that prerequisite even when installed explicitly.
- **Memory result (E03).** Before:
  `if governor.reconcile_memory(permit, bytes) { /* applied */ }`.
  After: `match governor.reconcile_memory(permit, bytes) {
  ReconcileOutcome::Applied { .. } => {},
  ReconcileOutcome::EpochExhausted { .. } => { /* release permit; no retry */ },
  _ => {} }`. Explicit `reconcile_memory_at` remains typed. `MeasurementSequence`
  increments checked; an exhausted permit does not change held memory or
  activity time and must still be released. Consumers matching
  `ReconcileOutcome` should handle exhaustion distinctly from stale samples.
- **Dispatch/admission behavior (H01/H02).** Invalid requested stack size now
  fails preflight before worker creation or engine admission; a valid size
  followed by OS spawn failure remains a distinct typed outcome. Immediate
  and queued acquisition use one handoff arbiter. Cancel wins a tie, then
  absolute deadline, then relative timeout; equality is expired. Relative
  `Duration::ZERO` is try-once but cannot suppress an expired absolute
  CompleteBy. A rejected handoff returns the unstarted permit exactly once.
- **Rayon constructors (H03).** `RayonCpuExecutor::new(n)` and
  `from_topology(&topology)` remain deprecated panic wrappers. Move fail-closed
  callers to `try_new(n)?` / `try_from_topology(&topology)?`; they return typed
  `RayonBuildError::{ZeroWorkers, InvalidTopology, Pool}`. No second engine
  worker pool is introduced.

### Verification status

- Product tickets C01, E01–E04, H01–H03 have owner-local tests. V01 shared
  receipt integration and V02 mutation producer are implemented but not
  hosted/full-denominator qualified. V03 producer has local semantic fuzz and
  model evidence; hosted replay remains separate.
- The exact `1378383` local `taskmesh-rayon` generated subset had 10 planned:
  5 caught, 0 missed, 5 unviable, 0 timeout/equivalent. This is **FAIL** under
  the current generated quality rule, not a workspace score. One unviable
  `try_new -> Ok(Default::default())` replacement cannot compile because the
  executor intentionally has no `Default`. Full workspace generated results
  remain NOT_RUN for this candidate.
- Coverage is descriptive, not a correctness threshold. Line/region/function/
  instantiation percentages and collected state are preserved separately;
  branch/MCDC count zero is `NOT_COLLECTED`, not 0% coverage.

## [0.2.0] - 2026-09-18

### Added

**Contract (`taskmesh-contract`, re-exported by `taskmesh`)**

- `ExecutorCapabilities` and a default-implemented `CpuExecutor::capabilities()`
  (D05). An adapter declares what it guarantees — `nonblocking_submit`,
  `declared_workers`, `exclusive_pool` — and the host never assumes more. The
  default is `ExecutorCapabilities::legacy()` (submission may block, worker count
  unknown, pool shared), so an existing `impl CpuExecutor` compiles unchanged.
- `TokioRuntime::executor_capabilities()` exposes the built runtime's adapter
  declaration.
- `TopologyError` (`InvertedCpuWorkerBounds`, `SlotCountTooLarge`,
  `ZeroFixedCpuWorkers`, `ExecutorDeclaresFewerWorkers`), `MAX_CAPABILITY_SLOTS`,
  `TopologyConfig::validate()`, `TopologyConfig::declared_slots()`, and
  `TopologyConfig::try_resolved_cpu_workers()` (TM16-003). `GovernorError::InvalidTopology`
  wraps it, `From<TopologyError> for GovernorError` and `Error::source()` are
  implemented.
- `Snapshot` wire schema 2 (D06): `SNAPSHOT_SCHEMA_VERSION`, `Snapshot::schema_version`,
  `Snapshot::capabilities` (per capability-pool `CapabilityUsage { in_use, limit }`),
  `ClassSnapshot` phase gauges `dispatch_reserved` / `accepted` / `running` /
  `cleanup_pending`, and cumulative `admitted_total` / `started_total` /
  `terminated_total`. `ClassSnapshot::conservation_violation()` and
  `Snapshot::conservation_violation()` check the published identities.
  `ClassSnapshot` now implements `Default`.
- `ExecutionPhase` (`DispatchReserved → Accepted → Running → CleanupPending`), the
  ownership phase of a live request; `rank()`, `has_started()`, `Display`.
- `AdmissionVerdict::SubstrateSaturated { retry_after_ms }` (D01): an immediate
  shed because the capability pool is full and there is no room to wait — the
  class does not queue, or its queue is already full while its head waits on
  that pool. Its `retry_after_ms` is reported by `AdmissionVerdict::retry_after_ms()`.
- `TaskSpec::awaited_child_of(root, parent_stage)` and
  `TaskScope::Child { parent_awaits }` (D12 revision): declares that the
  parent's execution blocks on this child. `AdmissionVerdict::NestedWaitCycle
  { held_by_root: HeldCapacity }` is what such a child gets, before admission
  and with nothing charged, when the capacity it would wait for — class
  inflight, a capability pool, the cpu or memory budget — is held entirely by
  its own root's permits: queueing it would be a deadlock by declaration, and
  no retry can help (`retry_after_ms()` is `None`). `child_of` is unchanged:
  lineage is not a wait, and undeclared waits are never inferred. Capacity
  shared with another root or with a sibling child is a wait, not a cycle.
  `HeldCapacity` (`ClassInflight { class }`, `CapabilityPool { pool }`,
  `CpuBudget`, `MemoryBudget`; `#[non_exhaustive]`) names the capacity.
- `TerminalReason` (`Reclaimed`, `Released`, `Abandoned`): why a queued ticket
  ended without its waiter taking ownership.
- `GovernorError` variants (all additive; the enum was and is `#[non_exhaustive]`):
  `TicketClaimTerminated { ticket, reason }`, `InvalidTicketClaim { ticket }`,
  `InvalidTopology(TopologyError)`, `DeadlineUnsupported { class, policy }`,
  `LeaseReclaimed { permit_id }`, `WorkerUnavailable { context, detail }`,
  `WorkerPanicked { context }`, `JobAbandoned { context }` (D14).
- `ResourceConversionError` (`UnscaledMemoryUnits`, `MemoryUnitsOverflow`), the
  typed error of the now-checked `MemoryUnitScale::units_for` (D06).
- `Clock` documentation now states the lock discipline the engine guarantees:
  `now_ms` is sampled *before* the state mutex is taken, never under it, and a
  commit watermark keeps lease timestamps monotonic (D07). The trait itself is
  unchanged.

**Engine (`taskmesh-engine`, via `taskmesh::ext`)**

- Typed outcomes for every transition that used to answer with a primitive
  (D14): `ClaimOutcome` (`Ready(PermitId)`, `Pending`, `Terminal(TerminalReason)`,
  `Invalid`), `ReleaseOutcome` (`Released`, `UnknownPermit`; `#[must_use]`),
  `StageReleaseOutcome` (`Released { freed_units }`, `PolicyForbids { policy }`,
  `UnknownPermit`, `StaleSequence { current_sequence }`; with `freed_units()` and
  `is_released()`), `ReconcileOutcome` (`Applied { held_units }`, `UnknownPermit`,
  `StaleEpoch { current_epoch }`, `ConversionFailed(ResourceConversionError)`;
  with `is_applied()`). `ClaimOutcome` and `ReleaseOutcome` are exhaustive (a
  waiter must handle every outcome); `StageReleaseOutcome` and
  `ReconcileOutcome` are `#[non_exhaustive]`.
- `Governor::reconcile_memory_at(permit, bytes, epoch) -> ReconcileOutcome`
  (epoch-ordered reconcile for concurrent reporters; the epoch is assigned inside
  the same transition that applies the reading, D15) and
  `Governor::release_stage_memory_seq(permit, units, sequence) -> StageReleaseOutcome`
  (replayed or reordered stage events are rejected, D02).
- `Governor::admit_resolved(spec, capability, waker)`: admit against an
  explicitly resolved capability pool (D04).
- `Governor::ticket_status(ticket) -> ClaimOutcome` (read-only; `claim` consumes).
- `Governor::advance_phase(permit, ExecutionPhase) -> AdvanceOutcome` and
  `Governor::phase(permit) -> Option<ExecutionPhase>` (monotonic; a backwards or
  repeated declaration is `Refused(AdvanceRefusal::NotLater { current })`, an
  unknown permit `Refused(AdvanceRefusal::UnknownPermit)`). The first advance
  out of `DispatchReserved` answers `Leased(LeaseToken)` — the permit's one
  proof of custody — and later advances `Advanced`.
- `LeaseToken` (not `Clone`; `#[must_use]`; `permit_id()`) and
  `Governor::release_leased(token) -> ReleaseOutcome` (D14 revision): the only
  way to end a dispatched permit. `ReleaseOutcome::HeldByLease { phase }` is
  what a plain `release(permit_id)` answers for a permit that has left
  `DispatchReserved`, and what `release_leased` answers for a token that is not
  that permit's lease (another governor's token for the same id, or a token for
  a permit that was never leased). Nothing moves on either refusal.
- Diagnostics: `PendingView` (`Governor::pending_view`), `PermitLedgerView`
  (`Governor::permit_ledger`, `Governor::permit_ledgers`) exposing
  `original_estimate_units` / `remaining_reservation_units` / `effective_units`
  (D02), `Governor::pending_block_reason -> Option<CapacityBlock>`,
  `Governor::accounting_fault()`, `Governor::retained_terminal_tickets()`.
- `CapacityBlock` (`Ok`, `Inflight`, `Capability`, `Cpu`, `Memory`,
  `QueuedBehind`, `AccountingFault`), `CapabilityName` (`Arc<str>`),
  `PROMOTION_BUDGET` (64 grants per promotion pass), `MAX_TERMINAL_TICKETS`
  (4096 retained terminal tickets).
- `PolicySet::with_capability_limits(BTreeMap<String, u32>)` — the single
  physical-capacity authority, checked in the same admission transition as class
  inflight and the resource budget (D01) — plus the accessors
  `PolicySet::substrates()`, `PolicySet::capability_limits()`,
  `PolicySet::capability_limit(pool)`, `PolicySet::capability_name(pool)`.
- `LeakSweepReport::retained_active`: stale leak-detecting permits the sweep
  observed but left charged because their work had already reached an executor
  (TM16-010). Stale is not dead.
- `RequestKey` is exported from `taskmesh-engine` and `taskmesh::ext` as the type
  of the read-only `PendingView::request_key` field, with `as_str()` and
  `Display`. It is *not* an input: no API accepts one, and it is derived
  authoritatively from `root_operation_id`.
- `taskmesh::MAX_REQUESTED_STACK_BYTES` (16 GiB): a `stack_size_bytes` above it
  is refused by the host with `PolicyViolation("requested stack size … exceeds
  the supported maximum …")` before `std::thread` sees it. Requests up to the
  maximum are handed to the OS as before (an OS refusal is `WorkerUnavailable`).
- `taskmesh::ext::TokioPermitWaker`: the host's `PermitWaker` adapter, for
  embedders that drive `Governor::admit_waitable` / `claim` themselves.
- `TokioRuntime::drain(timeout)` and `TokioRuntime::is_draining()` (D17), the
  host's graceful shutdown contract. `drain` closes admission in the engine
  (one-way; every later `run_*` / `_with` / requested-stack / dedicated-thread
  submission is refused *before admission* with
  `GovernorError::Rejected(AdmissionVerdict::RuntimeUnavailable)` — nothing
  queued, nothing charged) and then waits, event-driven, until every class
  reports `inflight == 0 && queued == 0` on the engine's own gauges. Work
  already queued or in flight is not cancelled. `Ok(DrainReport { elapsed,
  classes_drained })` on success; `Err(NotDrained { classes, elapsed })` when
  the timeout elapses first, listing each class's outstanding `Outstanding {
  inflight, queued }` — the runtime stays draining and a later `drain`
  continues the same wait. Dropping the handle remains the only teardown; a
  started blocking job cannot be aborted (D10), so `NotDrained` is what a
  shutdown sees when one is still running.
- `Governor::close_admission()` / `Governor::admission_closed()` (D17): the
  engine half of the drain, usable by embedders driving the governor directly.
  Decided under the admission lock, so a `snapshot()` taken after
  `close_admission` returns already holds every admission that will ever
  happen. `AdmissionVerdict::RuntimeUnavailable` now means "closed *or*
  accounting fault"; `admission_closed()` and `accounting_fault()` say which.
- `taskmesh::ext::RayonCpuExecutor` (with `features = ["rayon"]`): the Rayon
  adapter is reachable through the facade; a direct `taskmesh-rayon`
  dependency is no longer needed to size or share the pool.
- `#[non_exhaustive]` — a `match` needs a `_` arm — on `GovernorError`,
  `AdmissionVerdict`, `TerminalReason`, `ExecutionPhase`, `TopologyError`,
  `ResourceConversionError`, `StageReleaseOutcome`, `ReconcileOutcome`, and the
  struct `ExecutorCapabilities` (construct it from `legacy()` + builders, never
  as a literal). Exhaustive by design, matchable without a wildcard:
  `ClaimOutcome`, `ReleaseOutcome`, `CapacityBlock`, `AdmissionDecision`.
- `#[must_use]` on every outcome an embedder could drop by mistake:
  `AdmissionDecision`, `ClaimOutcome`, `ReleaseOutcome`, `StageReleaseOutcome`,
  `ReconcileOutcome`, `AdvanceOutcome`, and `LeaseToken`. A statement-form
  `governor.admit(&spec);` is now a warning, because a dropped `Admitted` is a
  leaked permit.
- `Debug` for `TokioRuntime`, `Builder`, and `Governor` (identity and policy
  summary; never the guarded state), `PartialEq`/`Eq` for `SubmissionDeadline`.
- `ExecutionPhase` and `TerminalReason` are re-exported at the engine root.
- `test-util` additions (not part of the consumer surface): `PolicySet::forge_substrates`,
  `Governor::drr_ring_visits`. New optional features `loom` and `shuttle` for
  the model-check builds (D13); they are off by default and resolve no extra
  dependency in a downstream lockfile.

**Facade (`taskmesh`)**

- Root re-exports: `CapabilityUsage`, `ExecutionPhase`, `TerminalReason`,
  `TopologyError`.
- `taskmesh::ext` re-exports: `ExecutorCapabilities`, `CapabilityName`,
  `CapacityBlock`, `ClaimOutcome`, `PendingView`, `PermitLedgerView`,
  `ReconcileOutcome`, `ReleaseOutcome`, `StageReleaseOutcome`,
  `MAX_TERMINAL_TICKETS`, `PROMOTION_BUDGET`.

**Rayon adapter (`taskmesh-rayon`)**

- `RayonCpuExecutor::capabilities()` declares `nonblocking_submit = true`, the
  pool's real thread count as `declared_workers`, and `exclusive_pool = true` for
  pools the adapter built (`try_new` / `from_topology`) or `false` for a pool
  handed in through `with_pool` (D05).

**Tooling and documentation**

- `tools/consumer-msrv` is now the migration fixture: it compiles *and runs*
  every migrated API shape in this changelog on the declared MSRV (Rust 1.81),
  with the default feature set and with `rayon`. `just consumer-msrv` fails
  when any documented outcome differs from what a consumer observes.
- `tools/doc-examples` compiles every ```rust block in `README.md`,
  `docs/taskmesh-external-interface.md` and this changelog against the facade
  (`cargo test --workspace`). A fence attribute picks the scaffold
  (`rust,body` / `rust,arms` / `rust,builder`); `rust,ignore` is allowed only
  here, and only on a block that opens with `// 0.1.0` — the one kind of
  unchecked code the docs may carry is a quotation of the previous release.
- Fuzzing (`fuzz/`, its own workspace): three libFuzzer targets over the
  production `Governor` (every public transition, invariants after each
  step, quiescence at the end), the policy / topology / builder front doors,
  and the JSON wire formats. `just fuzz` (nightly + cargo-fuzz; NOT_RUN
  without them, `FUZZ_SECONDS` per target) is a required proof gate;
  `just fuzz-check` type-checks and lints the targets on stable in the fast
  gate. The gate inventory has 24 gates.
- `docs/adr/0003-sep-16-hardening-contracts.md` records the contracts behind
  this release; `docs/release-checklist.md` gains the `cargo semver-checks`
  step.

### Changed

- **Admission is one transition (D01, TM16-001).** Semantic class capacity and
  the physical capability-pool slot are decided together, in the engine, under
  the class's own `OverflowPolicy`. The host-side substrate semaphore that
  requests used to wait on *before* admission is gone, so `Snapshot::queued`
  now counts every waiting request and `max_queue_depth` / `OverflowPolicy::Reject`
  govern the only queue there is.
- **Reservation, measurement, and release are separate facts (D02, TM16-005,
  TM16-011).** The permit ledger keeps `original_estimate_units`
  (provenance), `remaining_reservation_units` (shrinks on stage release, never
  grows), and `effective_units` (the current charge). `MemoryPermitMode::Estimated`
  charges the *remaining* reservation, so a reconcile after a stage release no
  longer resurrects the released units. `MemoryReleasePolicy` is enforced.
- **Aggregates are exact (D06, TM16-008).** Per-request cost stays `u32`;
  per-class and global held totals are `u128`; every capacity comparison is
  checked. An unrepresentable total is `CapacityBlock::AccountingFault` (the
  engine refuses new work) and is visible through `Governor::accounting_fault()`.
  `MemoryUnitScale::units_for` returns `Result` instead of saturating.
- **Clock is read outside the lock (D07, TM16-026).** The engine samples `Clock`
  before taking its state mutex and clamps the committed time to a monotonic
  watermark. A custom `Clock` is never called under the transition lock and no
  longer needs `now_ms` itself to be monotonic for lease timestamps to be.
- **Fairness picks fully runnable candidates (D08).** A class is a scheduling
  candidate only when both its semantic capacity and the capability pool frozen
  at intake are available; strict FIFO within a class is preserved
  (`CapacityBlock::QueuedBehind`). WFQ fixed-point scale is `2^64` so no
  admissible weight (`1..=u32::MAX`) yields a zero increment (TM16-013). DRR
  ring traversal is arithmetic, `O(classes)` per selection (TM16-012).
- **Acquisition budget covers every wait (D09, TM16-032).** `SubmitOptions::acquire_timeout`
  includes admission-lock wait. A permit that arrives after the budget expired —
  immediately or via a promotion racing the timeout — is returned unstarted and
  the caller gets `PermitAcquireTimedOut` (or `SubstratePoolTimedOut`).
  `Some(Duration::ZERO)` still means "try, do not wait".
- **Response is not custody (D10, TM16-015, TM16-024).** A deadline reply, a
  cancel, or a dropped caller future ends the caller's wait; a synchronous
  worker (blocking, CPU, requested-stack) keeps its execution lease until it
  actually terminates, including tearing down an owned runtime. On normal
  completion custody moves with the result, so capacity is already released
  when the caller sees the value. The snapshot reports such a request as
  `running` / `cleanup_pending` until then.
- **`run_blocking` honours `RunFor` as a caller-wait bound (D10, TM16-002).**
  Previously the deadline on the blocking path was inert. A started blocking job
  is not aborted; the caller's wait is bounded and `DeadlineExceeded` is returned.
- **CPU `RunFor` is anchored to the worker's own start (D05, TM16-022).** The
  budget starts when the job starts on the worker, not when `spawn` returned, so
  an inline executor cannot return a late success as `Ok`. Ties go to the deadline.
- **Stack requests occupy `large_stack` (D04, TM16-023).** A blocking-family
  submission with `stack_size_bytes` reserves the `large_stack` capability
  regardless of its declared hint, so `large_stack_slots` actually caps dedicated
  workers.
- **Topology is resolved once (D05).** `Builder::build` reads detected
  parallelism once and sizes the `cpu` capability gate and the default executor
  from that single answer. `TopologyConfig::resolved_cpu_workers` is now total
  (an inverted window resolves instead of panicking); prefer
  `try_resolved_cpu_workers` on construction paths.
- **`PolicySet::default()` seeds the built-in inventory (TM16-004).** It now
  delegates to `PolicySet::new`, so a defaulted policy set carries `cpu`,
  `blocking`, `large_stack`, `maintenance`, `local_runtime`. `Governor::new`
  validates the registry as a whole (presence and binding of the built-ins,
  key/record agreement) regardless of how it was assembled.
- **Lease activity is recorded in the transition that proves it (D15,
  TM16-009).** `claim` touches the lease (the sweep's staleness clock restarts
  at claim, not at promotion) and a successful stage release counts as a
  heartbeat. Terminal ticket retention is bounded (`MAX_TERMINAL_TICKETS`) and
  indexed.
- **Dedicated-thread labels are derived, not interpolated (TM16-031).** The OS
  thread name for a requested-stack worker is a bounded, sanitised label; a class
  name containing NUL or non-ASCII no longer panics inside `thread::Builder`.
  Class identity itself is unchanged.
- **Waker destructors run outside the governor lock (TM16-030).** Host
  `PermitWaker` values are retired after the transition, so a waker `Drop` that
  re-enters the governor cannot deadlock.
- `LeakSweepReport::reclaimed_permits` can now be smaller than
  `suspected_leaks`: the sweep still counts every stale leak-detecting permit as
  suspected, but reclaims only those that never reached an executor; the rest
  are reported in the new `retained_active` (TM16-010).
- `taskmesh-engine`'s `loom` / `shuttle` dependencies moved from cfg-gated
  dev-dependencies to cfg-gated *optional* dependencies behind the features of
  the same name (D13).

### Removed

- `PolicySet::substrates` as a public field. Read the registry through
  `PolicySet::substrates()`; add records through `with_substrates`. `PolicySet`
  can no longer be built with a struct literal or `..Default::default()`
  functional-update syntax (it gained private fields); use `PolicySet::new`.
- The unbounded host-side wait in front of admission (D01). A non-queueing
  class whose capability pool is full is rejected immediately with
  `SubstrateSaturated`; it no longer parks until a slot frees.
- The silent drop of `SubmitOptions::deadline` on a class that cannot enforce
  it (the `deadline_ignored_for_plain_cooperative` behaviour). It is now
  `GovernorError::DeadlineUnsupported`.
- The `PolicyViolation(String)` encoding of worker failures ("cpu worker
  panicked", "… dropped result", "large-stack thread spawn failed", "requested-stack
  async runtime initialization failed"). See `WorkerPanicked`, `JobAbandoned`,
  `WorkerUnavailable`.
- The saturating (`u32::MAX`) and unscaled (`0`) results of
  `MemoryUnitScale::units_for`; both are now typed errors.
- Silent coercion of `FairnessPolicy::WeightedFairQueue { weight: 0, .. }` to
  weight 1; weight `0` is rejected at construction.
- Service debt for cancelled requests and idle credit accumulation in the
  fairness schedulers (TM16-014, TM16-037): a class that drained its queue does
  not carry credit into its next arrival, and a cancelled request leaves no debt.

### Fixed

- TM16-001: `OverflowPolicy::Reject` classes could accumulate unbounded waiters
  in the substrate gate while `queued` reported `0`.
- TM16-002: `run_blocking` + `RunFor` deadline was inert.
- TM16-003: an inverted `min_workers > max_workers` window or an oversized slot
  count panicked in a clamp / semaphore constructor instead of failing `build()`.
- TM16-004: `PolicySet::default()` produced an empty substrate registry that
  bypassed inventory validation.
- TM16-005: `MemoryReleasePolicy` was never read; `OnTaskCompletion` classes
  accepted stage releases.
- TM16-008: `saturating_add` in capacity checks let two `2^31`-unit requests
  both admit against a `2^32 - 1` budget.
- TM16-009 / TM16-010: stage activity did not touch the leak lease; the leak
  sweep could reclaim a permit whose work was already running.
- TM16-011: an `Estimated` reconcile after a stage release restored the
  original reservation.
- TM16-012: DRR selection iterated `O(cost / quantum)` under the lock, up to
  `u32::MAX` times for admissible policies.
- TM16-013: WFQ produced a zero virtual-time increment for large weights.
- TM16-014 / TM16-037: WFQ cancelled-service debt and DRR idle credit let past
  history decide the order of identical queue states. Cross-pool WFQ followers
  now charge only their actual service and immediately rebuild survivor tags,
  so an unrelated cancellation cannot reorder an older blocked request. Normal
  WFQ head service keeps the already-canonical tail and runs in `O(1)` under
  the governor lock instead of rescanning every survivor.
- `Builder::capability_limit` could override a topology-owned built-in pool when
  its topology slot count was the unbounded `0` sentinel, leaving portable
  config and the governor capacity ledger in disagreement.
- TM16-015: requested-stack workers released their lease *after* sending the
  result, leaving a phantom `inflight` visible right after completion.
- TM16-022: the CPU relative deadline started after `spawn` returned.
- TM16-023: stack requests were gated by the declared hint's pool, not
  `large_stack`, so `large_stack_slots = 1` allowed two dedicated workers.
- TM16-024: an owned runtime's teardown ran before the deadline response, so a
  blocking child could delay the caller indefinitely.
- TM16-026: lease timestamps were sampled before the lock and committed
  unclamped, creating leases that were stale on creation.
- TM16-030: a `PermitWaker` destructor ran under the governor lock.
- TM16-031: a class name with NUL panicked thread construction outside the
  worker's unwind boundary.
- TM16-032: `acquire_timeout` did not cover the admission-lock wait, and a
  permit granted after expiry started the job anyway.
- `MemoryOvercommitPolicy::DegradeToLight` pointing at a disabled fallback
  (`max_inflight = 0`) shed every degraded request as `ClassDisabled` naming the
  wrong class; it is now rejected at construction (D14). A fallback that itself
  declares `DegradeToLight` (a chain `a → b → c` or a cycle `a → b → a`) promised
  a second hop the engine never takes; it is rejected at construction too (D03).
- A panic in the root future of a requested-stack async worker is reported to
  the caller *before* the owned runtime is torn down (D10 revision).

### Breaking changes and migration

Every entry below is either a signature change, a removed item, or a behaviour a
0.1.0 consumer could have relied on. `cargo semver-checks` (0.50.0, baseline
`9ae9547`) reports the struct-field items; the return-type, field-type, and
behavioural items are invisible to it and were taken from a manual diff of the
public surface and from the contract tests in `crates/*/tests/hardening_*.rs`.

#### `Governor::claim` returns `ClaimOutcome` (was `Option<PermitId>`)

`None` used to mean both "not promoted yet" and "the permit this ticket held was
reclaimed"; a waiter treating the second as the first waited forever. All four
outcomes are terminal-or-not by construction (`Invalid` is terminal).

```rust,ignore
// 0.1.0
loop {
    if let Some(permit) = governor.claim(ticket) {
        break permit;
    }
    waker.notified().await;
}
```

```rust,body
// 0.2.0 — the waker is the host adapter, registered at admission
use std::sync::Arc;
use taskmesh::ext::{AdmissionDecision, ClaimOutcome, PermitWaker, TokioPermitWaker};

let waker = TokioPermitWaker::new(); // Arc<TokioPermitWaker>
let port: Arc<dyn PermitWaker> = waker.clone();
let ticket = match governor.admit_waitable(&spec, port) {
    AdmissionDecision::Queued { ticket } => ticket,
    AdmissionDecision::Admitted { permit_id } => return Ok(permit_id),
    AdmissionDecision::Rejected(verdict) => return Err(GovernorError::Rejected(verdict)),
};
loop {
    match governor.claim(ticket) {
        ClaimOutcome::Ready(permit) => break Ok(permit),
        ClaimOutcome::Pending => waker.notified().await,
        ClaimOutcome::Terminal(reason) => {
            break Err(GovernorError::TicketClaimTerminated {
                ticket,
                reason: reason.into(),
            })
        }
        ClaimOutcome::Invalid => break Err(GovernorError::InvalidTicketClaim { ticket }),
    }
}
```

An embedder that drives the engine this way also owes the ownership phases:
`governor.advance_phase(permit, ExecutionPhase::Running)` before the work and
`CleanupPending` after. The leak sweep reclaims only `DispatchReserved` permits,
so a permit that is never advanced is *reclaimable while its work runs* once
`DEFAULT_LEAK_STALE_MS` (60 000 ms) passes on a `LeakDetecting` class; the phase
gauges and `started_total` are meaningless without the declarations. The first
advance hands back the permit's `LeaseToken`; keep it with the work and end the
permit with `release_leased(token)` (see the next section).

`claim` consumes the ticket; use `Governor::ticket_status(ticket)` for a
read-only look.

#### `Governor::release` returns `ReleaseOutcome` (was `()`), `#[must_use]`

`UnknownPermit` is a double release (host bug) or a lease the leak sweep already
reclaimed (a race the host must notice). `HeldByLease { phase }` is a release by
id of a permit that has been dispatched: its `LeaseToken` owns it now (next
section). A statement-form call now warns.

```rust,ignore
// 0.1.0
governor.release(permit);
```

```rust
// 0.2.0
use taskmesh::ext::ReleaseOutcome;
match governor.release(permit) {
    ReleaseOutcome::Released => {}
    ReleaseOutcome::UnknownPermit => {
        // before dispatch: the sweep reclaimed a stale DispatchReserved lease;
        // after dispatch: a double release — treat as a bug.
    }
    ReleaseOutcome::HeldByLease { phase } => {
        // the permit was advanced past DispatchReserved: only its LeaseToken
        // releases it. Nothing changed.
        let _ = phase;
    }
}
// or, where a double release is impossible by construction:
assert_eq!(governor.release(permit), ReleaseOutcome::Released);
```

#### `Governor::advance_phase` returns `AdvanceOutcome` (was `bool`); dispatched permits are released by `LeaseToken`

Naming a permit is no longer owning it. `runtime.governor()` is public and permit
ids are sequential, so a plain `release(permit_id)` could refund a *running*
job's capacity under it by passing the wrong number; the runtime's own lease
only found out later (`UnknownPermit`, a debug assertion). Now the first advance
out of `DispatchReserved` mints the permit's one `LeaseToken`, and from then on
only `release_leased(token)` ends the permit. The token is not `Clone` and is
`#[must_use]`: dropped unspent, its permit stays charged for good (the leak
sweep never reclaims a leased permit). Undispatched permits — a direct `admit`
an embedder never advances — are still released by id.

```rust,ignore
// 0.1.0
assert!(governor.advance_phase(permit, ExecutionPhase::Running));
run(job);
governor.release(permit);
```

```rust
// 0.2.0
use taskmesh::ext::{AdvanceOutcome, ReleaseOutcome};
let token = match governor.advance_phase(permit, ExecutionPhase::Running) {
    AdvanceOutcome::Leased(token) => token, // keep it with the work
    AdvanceOutcome::Advanced => unreachable!("a permit is leased on its first advance"),
    AdvanceOutcome::Refused(refusal) => {
        // reclaimed before dispatch, or a phase that is not later: not started
        return Err(format!("not started: {refusal:?}").into());
    }
};
run(job);
assert_eq!(governor.release_leased(token), ReleaseOutcome::Released);
```

#### `Governor::release_stage_memory` returns `StageReleaseOutcome` (was `u32`)

A policy refusal was folded into "freed 0 units". Stage release now requires the
class to declare `MemoryReleasePolicy::OnStageBoundary` or `LeakDetecting`;
`OnTaskCompletion` (the default) answers `PolicyForbids`.

```rust,ignore
// 0.1.0
let freed: u32 = governor.release_stage_memory(permit, 2);
```

```rust
// 0.2.0
use taskmesh::ext::StageReleaseOutcome;
match governor.release_stage_memory(permit, 2) {
    StageReleaseOutcome::Released { freed_units } => { /* freed_units <= 2 */ }
    StageReleaseOutcome::PolicyForbids { policy } => {
        // the class's MemoryReleasePolicy does not allow stage release
    }
    StageReleaseOutcome::UnknownPermit => { /* no such live permit */ }
    StageReleaseOutcome::StaleSequence { current_sequence } => {
        // only with release_stage_memory_seq: a replayed / reordered event
    }
    _ => {} // the enum is non_exhaustive
}
// `.freed_units()` gives the 0.1.0 number back when that is all you need.
```

Ordered reporters should use `release_stage_memory_seq(permit, units, sequence)`
with a strictly increasing `sequence`.

#### `Governor::reconcile_memory_at` → `ReconcileOutcome` (new; `reconcile_memory` keeps `bool`)

Concurrent reporters need an order. `reconcile_memory(permit, bytes) -> bool` is
unchanged and assigns the next epoch itself; `reconcile_memory_at` takes an
explicit epoch and reports why a reading was not applied. A stage-released
reservation is no longer resurrected by an `Estimated` reconcile (D02).

```rust
use taskmesh::ext::ReconcileOutcome;
match governor.reconcile_memory_at(permit, measured_bytes, epoch) {
    ReconcileOutcome::Applied { held_units } => { /* new effective charge */ }
    ReconcileOutcome::StaleEpoch { current_epoch } => { /* a newer reading won */ }
    ReconcileOutcome::UnknownPermit => {}
    ReconcileOutcome::ConversionFailed(error) => { /* bytes not expressible in units */ }
    _ => {}
}
```

#### `Snapshot` wire schema 2

`Snapshot` and `ClassSnapshot` are serde types. A 0.1.0 JSON snapshot does not
deserialize as 0.2.0 and vice versa. Changes:

| Field | 0.1.0 | 0.2.0 |
|---|---|---|
| `Snapshot.schema_version` | — | `2` (`taskmesh::SNAPSHOT_SCHEMA_VERSION`, also in `taskmesh_contract`) |
| `Snapshot.capabilities` | — | `BTreeMap<String, CapabilityUsage { in_use: u32, limit: u32 }>`; `limit == 0` means ungated |
| `ClassSnapshot.cpu_units_held`, `memory_units_held` | `u32`, JSON number | `u128`, JSON **decimal string** (`"6"`) |
| `ClassSnapshot.dispatch_reserved`, `accepted`, `running`, `cleanup_pending` | — | `u32` phase gauges; they partition `inflight` |
| `ClassSnapshot.admitted_total`, `started_total`, `terminated_total` | — | `u128`, JSON decimal string |

Conservation identities you may assert on any snapshot:
`inflight == dispatch_reserved + accepted + running + cleanup_pending`,
`admitted_total == inflight + terminated_total`,
`started_total >= running + cleanup_pending`, and for every capability
`in_use <= limit` when `limit != 0`. `Snapshot::conservation_violation()` checks
them all.

```rust,ignore
// 0.1.0
let held: u32 = snapshot.classes[&class].cpu_units_held;
```

```rust
// 0.2.0
assert_eq!(snapshot.schema_version, 2);
let held: u128 = snapshot.classes[&class].cpu_units_held;
let cpu_pool = &snapshot.capabilities["cpu"]; // (in_use, limit)
if let Some(violation) = snapshot.conservation_violation() {
    // the projection is inconsistent — report it
}
```

A JSON consumer must parse `cpu_units_held` / `memory_units_held` /
`*_total` as strings and convert; reading them as numbers is what schema 2
exists to prevent (a large `u128` does not survive an `f64`). Struct-literal
construction of `ClassSnapshot` / `Snapshot` needs the new fields, or use
`..Default::default()` (both types implement `Default`).

`RootAttribution::cpu_units` / `memory_units` are likewise `u128` (were `u32`).

#### `GovernorError` new variants; `PolicyViolation` no longer means "worker failed"

`GovernorError` is `#[non_exhaustive]`, so a `match` with a `_` arm keeps
compiling. What silently changes is *which arm fires*: in 0.1.0 a panicking or
vanished worker was `PolicyViolation("cpu worker panicked")` and similar
strings. In 0.2.0:

| Situation | 0.1.0 | 0.2.0 |
|---|---|---|
| job or adapter `spawn` panicked | `PolicyViolation("… panicked")` (CPU / dedicated thread) or `PolicyViolation("spawn_blocking join failure")` (blocking pool) | `WorkerPanicked { context }` |
| thread / owned runtime could not be created | `PolicyViolation("… spawn failed: …")` | `WorkerUnavailable { context, detail }` (job never started) |
| adapter dropped the closure; worker vanished | `PolicyViolation("… dropped result")` | `JobAbandoned { context }` |
| leak sweep reclaimed the lease before dispatch | not reported | `LeaseReclaimed { permit_id }` (job never started) |
| queued ticket ended without ownership | `None` from `claim` | `TicketClaimTerminated { ticket, reason }` / `InvalidTicketClaim { ticket }` |
| impossible topology | panic | `InvalidTopology(TopologyError)` |
| deadline on a class that cannot enforce it | silently ignored | `DeadlineUnsupported { class, policy }` |

`PolicyViolation(Cow<str>)` remains for host configuration/usage errors only
(`CompleteBy` on a blocking path, missing or oversized stack size, `Instant`
overflow, non-monotonic phase declaration, policy validation failures). A
consumer that matched `PolicyViolation` to detect worker failure must add arms
for the new variants:

```rust,ignore
// 0.1.0 (the blocking pool said "spawn_blocking join failure", so this arm never
// caught a blocking-pool panic — one more reason the strings were not a contract)
Err(RunError::Governor(GovernorError::PolicyViolation(msg))) if msg.contains("panicked") => retry(),
```

```rust,arms
// 0.2.0
Err(RunError::Governor(GovernorError::WorkerPanicked { .. })) => { /* side effects unknown */ }
Err(RunError::Governor(GovernorError::WorkerUnavailable { .. })) => retry(), // never started
Err(RunError::Governor(GovernorError::JobAbandoned { .. })) => { /* side effects unknown */ }
Err(RunError::Governor(GovernorError::LeaseReclaimed { .. })) => retry(),    // never started
Err(RunError::Governor(GovernorError::PolicyViolation(msg))) => { /* fix the call site */ }
Err(RunError::Governor(_)) => {}
```

#### `SubmitOptions::deadline` on a non-`CooperativeWithDeadline` class → `DeadlineUnsupported`

In 0.1.0 `with_deadline` / `with_absolute_deadline` on a class whose
`cancellation_policy` was `PreSubmitOnly` or `Cooperative` was dropped and the
work ran to completion returning `Ok`. In 0.2.0 the submission is refused before
admission (nothing is admitted, nothing is charged). Either declare the policy
or stop passing a deadline:

```rust,builder
// 0.2.0 — the class must be able to enforce what the submission asks for
.class_policy(
    TaskClass::new("fetch"),
    ClassPolicy::new()
        .max_inflight(8)
        .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
)
```

```rust
// …
match runtime.run_io_with(spec, SubmitOptions::unbounded().with_deadline(d), fut).await {
    Err(RunError::Governor(GovernorError::DeadlineUnsupported { class, policy })) => {
        // configuration error: `class` has `policy`, which cannot enforce a deadline
    }
    Err(RunError::Governor(GovernorError::DeadlineExceeded)) => { /* enforced */ }
    other => {}
}
```

Blocking-path `RunFor` now bounds the *caller's wait* (the started job is not
aborted). `CompleteBy` on a blocking path is still `PolicyViolation`.

#### `TaskScope::Child` gained `parent_awaits`

An exhaustive pattern `TaskScope::Child { parent_stage }` no longer compiles;
write `TaskScope::Child { parent_stage, .. }`. On the wire the field has a
default, so a payload written before it existed still parses (as an undeclared
wait). `child_of` still builds an undeclared child; `awaited_child_of` is new.

```rust,ignore
// 0.1.0
match spec.scope {
    TaskScope::Root => {}
    TaskScope::Child { parent_stage } => attribute(parent_stage),
}
```

```rust
// 0.2.0
match &spec.scope {
    TaskScope::Root => {}
    TaskScope::Child { parent_stage, .. } => attribute(parent_stage),
}
```

#### `SubstrateSaturated` vs `SubstratePoolTimedOut`

A class that does not queue (`OverflowPolicy::Reject`, the default) whose
capability pool (`blocking`, `large_stack`, `maintenance`, `local_runtime`,
`cpu`) is full is now rejected immediately with
`AdmissionVerdict::SubstrateSaturated { retry_after_ms }`. In 0.1.0 it waited
in an unbounded host semaphore and either ran eventually or, with an
`acquire_timeout`, came back as `SubstratePoolTimedOut`. `SubstratePoolTimedOut`
is now only the expiry of a *bounded* wait by a queueing class. A queueing class
whose queue is *full* while blocked on a pool is also shed as
`SubstrateSaturated` (not `QueueFull`): the verdict names the resource, and the
full queue is why it could not wait for it.

```rust,body
// 0.2.0 — keep the "eventually runs" behaviour by opting into a queue:
ClassPolicy::new()
    .max_inflight(8)
    .max_queue_depth(64)
    .overflow_policy(OverflowPolicy::QueueWithinDepth)
```

```rust,arms
// …and handle both verdicts:
Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateSaturated { retry_after_ms }))) => {
    // immediate shed: the pool is full and this class does not queue
}
Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstratePoolTimedOut { .. }))) => {
    // a queueing class's bounded wait expired
}
```

`Snapshot::queued` now includes every waiting request, and
`Snapshot::capabilities[pool]` shows the occupancy that caused the shed.

#### `ExecutorCapabilities` / `ExecutorDeclaresFewerWorkers`

`CpuExecutor::capabilities()` has a default (`legacy()`), so every existing
adapter compiles. **But** `Builder::build` now compares an adapter's
`declared_workers` with the `cpu` gate the topology resolved, and refuses a
declaration that is smaller:

```rust
// A custom adapter that declares its pool:
impl CpuExecutor for MyPool {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) { self.pool.execute(work) }
    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(self.pool.threads() as u32)
            .exclusive_pool(true)
    }
}

// 0.2.0: topology asks for 4, adapter has 2 → typed refusal at build()
let err = Builder::new()
    .topology(TopologyConfig::new().cpu_fixed(4))
    .cpu_executor(Arc::new(MyPool::with_threads(2)))
    /* … */
    .build()
    .unwrap_err();
assert!(matches!(
    err,
    GovernorError::InvalidTopology(TopologyError::ExecutorDeclaresFewerWorkers { declared: 2, resolved: 4 })
));
```

Fix by sizing the adapter from the same number (`TopologyConfig::try_resolved_cpu_workers`)
or by declaring at least the gate. An adapter that leaves `declared_workers`
unset (`None`, the legacy default, and `BlockingPoolCpuExecutor`) is *not*
refused — unknown is not "fewer" — and is not upgraded to a guarantee either.
`RayonCpuExecutor::with_pool` declares the pool as shared. Read the built
runtime's declaration with `TokioRuntime::executor_capabilities()`.

#### `TopologyConfig::validate` — impossible topologies are `Err`, not panics

`Builder::build` validates the topology before sizing anything:
`min_workers > max_workers`, `CpuMode::Fixed(0)`, and any slot count above
`MAX_CAPABILITY_SLOTS` (`u32::MAX`) return
`GovernorError::InvalidTopology(TopologyError::…)`. In 0.1.0 these panicked in
`clamp` or in the semaphore constructor. `TopologyConfig::validate()` is
public for callers that check deserialized input before building:

```rust
let topology: TopologyConfig = serde_json::from_str(input)?;
topology.validate()?;                       // TopologyError
let workers = topology.try_resolved_cpu_workers(available)?;
```

#### `RequestKey` is not a public input

No `taskmesh` API accepts an admission key. The key is derived from
`TaskSpec::root_operation_id` inside the engine; `RequestKey` appears in 0.2.0
only as the read-only `PendingView::request_key` diagnostic field
(`taskmesh_engine::RequestKey`). Nothing to migrate unless code was
constructing one to pass somewhere — it never had an effect.

#### `PolicySet` substrate registry: field → accessor; no struct literal

```rust,ignore
// 0.1.0
let records = policy.substrates.values();
let custom = PolicySet { resources, classes, substrates: my_map };
```

```rust
// 0.2.0
let records = policy.substrates().values();
let custom = PolicySet::new(resources, classes)
    .with_substrates(my_records)?               // duplicate / shadowing built-ins rejected
    .with_capability_limits(limits)?;           // limits for unregistered pools rejected
```

`PolicySet::default()` now contains the built-in inventory (0.1.0: an empty
registry).

#### Other behavioural changes a 0.1.0 consumer might have relied on

- `MemoryUnitScale::units_for(bytes)` returns `Result<u32, ResourceConversionError>`
  (was `u32`, `0` when unscaled, `u32::MAX` on overflow). `Measured` / `Hybrid`
  reconciles that hit either edge report `ReconcileOutcome::ConversionFailed`.
- `FairnessPolicy::WeightedFairQueue { weight: 0, burst: _ }` and
  `MemoryOvercommitPolicy::DegradeToLight` to a disabled fallback — or to a
  fallback that itself degrades (a degrade is one hop) — are rejected by
  `Builder::build` / `Governor::new` (`PolicyViolation`).
- `acquire_timeout` includes the admission-lock wait; a permit granted after the
  budget expired is unwound and reported as `PermitAcquireTimedOut`, not started.
- `run_blocking` honours a cancel token on a `Cooperative` /
  `CooperativeWithDeadline` class as a **caller-wait** cancellation: the caller
  gets `GovernorError::Cancelled` when the token fires, the started job keeps
  running and stays charged until it returns (D10). In 0.1.0 `run_blocking`
  never raced the token and always returned the job's result; a consumer that
  passed a token to a blocking job on such a class now sees `Cancelled` instead.
  `PreSubmitOnly` classes are unchanged (pre-submit only).
- `Snapshot::capabilities` always contains every registered pool (`cpu`,
  `blocking`, `large_stack`, `local_runtime`, `maintenance`, plus registered
  extras): gated pools with their limit, ungated pools with `limit == 0` and
  the live `in_use`. Before, an ungated pool was a key only while a job held it,
  so `capabilities["maintenance"]` could panic on an idle runtime and a
  dashboard saw keys appear and vanish.
- After a deadline / cancel / dropped future, a synchronous worker stays charged
  (`running` / `cleanup_pending`) until it finishes; do not expect `inflight == 0`
  immediately after an `Err(DeadlineExceeded)` on `run_blocking` / `run_cpu`.
- A blocking-family `TaskSpec` with `stack_size_bytes` counts against
  `large_stack_slots`, whichever hint it declares.
- `LeakSweepReport` gained `retained_active`; struct-literal construction needs
  it (or `..Default::default()`).
- `Governor::reap_leaks` reclaims only permits that never reached an executor;
  a stale lease on running work is retained and counted in `retained_active`.

#### Cross-check against `cargo semver-checks`

| Crate | Checks | Failing | Items |
|---|---|---|---|
| `taskmesh-contract` | 196 | 1 (`constructible_struct_adds_field`) | `ClassSnapshot.{dispatch_reserved, accepted, running, cleanup_pending, admitted_total, started_total, terminated_total}`, `Snapshot.{schema_version, capabilities}` — all covered under *Snapshot wire schema 2* |
| `taskmesh-engine` | 196 | 4 (`constructible_struct_adds_field`, `constructible_struct_adds_private_field`, `struct_pub_field_missing`, `struct_pub_field_now_doc_hidden`) | `LeakSweepReport.retained_active`; `PolicySet.{capability_limits, capability_names}` (private), `PolicySet.substrates` (removed / now private) — covered under *PolicySet* and *Other behavioural changes* |
| `taskmesh` | 196 | 0 | the facade only re-exports; the tool does not analyse cross-crate re-exports, so every contract/engine item above is reachable through `taskmesh` / `taskmesh::ext` regardless |
| `taskmesh-rayon` | 196 | 0 | `capabilities()` is an additive trait-method implementation |

Not flagged by the tool (it does not compare types): the return-type changes of
`Governor::claim` / `release` / `release_stage_memory` and
`MemoryUnitScale::units_for`; the `u32 → u128` widening of
`ClassSnapshot.{cpu,memory}_units_held` and `RootAttribution.{cpu,memory}_units`;
the serde representation change; and every behavioural entry in this section.
No tool finding is unmapped.

## [0.1.0] - 2026-07-19

Initial workspace version: `taskmesh-contract`, `taskmesh-engine`, `taskmesh`,
`taskmesh-rayon`. The 0.2.0 comparison baseline is commit `9ae9547` (the last
commit at version 0.1.0); the date is that commit's.

[0.2.0]: https://github.com/josongmin/quanta-taskmesh/compare/9ae9547...hardening/sep-16
[0.1.0]: https://github.com/josongmin/quanta-taskmesh/tree/9ae9547
