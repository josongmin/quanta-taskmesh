# Taskmesh engine status

Authority: **code**. Rechecked against `main@85f0042` on 2026-10-05.
Nightly/release qualification, Semantica D1–D9, and performance series remain
separate: see [operating kernel](README.md). `nightly` is an explicit high-cost
profile, not a schedule.

## Live governor

`taskmesh-engine::Governor`: `admit*` / `claim` / `abandon` / `release` /
`release_leased` / `advance_phase` / `reconcile_memory*` / `reap_leaks*` /
`promote` (`PROMOTION_BUDGET = 64`).

Fairness `select` (`features/fairness/scheduler.rs`) is not a FIFO fallback:

- FIFO / `BestEffortScavenger` → minimum physical `head_queue_order`, assigned
  under the state lock at enqueue (identity sequences may commit out of order)
- WFQ → `head_finish_tag` (virtual finish), then `head_queue_order`; **`burst`
  field unread**
- `DeadlineAware { slack_ms }` → `enqueued_at + slack` (saturating), then
  `head_queue_order` on ties; distinct from host `CompleteBy`
- DRR → arithmetic ring + idle `deficit = 0`

On capacity overflow, `DropBestEffort` rejects like `Reject`
(`is_queueable = false`); it does not silently drop or evict work. A separate
memory-overcommit queue policy can still queue. `CheckpointPolicy` is preserved
metadata exposed by `Governor::checkpoint_policy`; the host has no enforcement loop.

Tokio host (`taskmesh::TokioRuntime`) owns `ExecutionLease` /
`Custody::{Reserved,Leased,Gone}`. Engine has no Tokio.

## Host deadline

Synchronous `CompleteBy` is refused before a lease with `PolicyViolation`.
`DeadlineUnsupported` instead rejects a deadline on a class without
`CooperativeWithDeadline`. Cooperative async deadline adapters live in the
host; synchronous `run_*_response_by` bounds the caller response while a started
worker retains its lease until termination.

## Proof surface in code

Five Loom and seven Shuttle test definitions drive the production `Governor`
through `src/sync.rs`; their presence is not an execution verdict. A
differential model test is also in `taskmesh-engine`. See a source-bound gate
receipt for executed proof.
