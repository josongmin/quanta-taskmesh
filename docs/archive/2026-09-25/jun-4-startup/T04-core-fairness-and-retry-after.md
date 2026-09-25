# T04 Core Fairness and Retry-After

## Summary

fairness algorithm과 retry-after policy를 실제 core scheduling semantics로 구현한다. enum만 있는 상태를 끝낸다.

## Decisions Frozen

1. fairness는 core responsibility다.
2. retry-after는 governor decision에서 계산한다.
3. best-effort는 interactive workload를 starving 시키지 않는다.

## Files To Touch

1. `crates/taskmesh-core/src/lib.rs`
2. `crates/taskmesh-core/tests/fairness_fifo.rs` 신규
3. `crates/taskmesh-core/tests/fairness_weighted.rs` 신규
4. `crates/taskmesh-core/tests/fairness_best_effort.rs` 신규
5. `crates/taskmesh-core/tests/retry_after.rs` 신규

## Implementation

1. `Fifo` scheduling 구현
2. `WeightedFairQueue` 구현
3. `DeficitRoundRobin` 구현
4. `DeadlineAware` 구현
5. `BestEffortScavenger` 구현
6. retry-after policy 구현
   - `None`
   - `FixedMs`
   - `Adaptive`
7. `Adaptive`는 deterministic testable heuristic으로 구현한다.
   - current queue depth
   - current inflight
   - class weight/quantum/slack
8. public surface는 그대로 두고 algorithm-specific helper type을 core private로 둔다.

구체 선택:

1. `Fifo`
   - algorithm: arrival-order `VecDeque`
2. `WeightedFairQueue`
   - algorithm: virtual finish time
   - data structure: `BinaryHeap<Reverse<(u128, u64, TaskClass)>>`
   - tag formula: `finish_tag = max(prev_finish, virtual_time) + cost / weight`
3. `DeficitRoundRobin`
   - algorithm: deficit counters
   - data structure: `BTreeMap<TaskClass, DeficitState>` + `VecDeque<TaskClass>`
4. `DeadlineAware`
   - algorithm: earliest deadline first with stable seq tie-break
   - data structure: `BinaryHeap<Reverse<(u64, u64, TaskClass)>>`
5. `BestEffortScavenger`
   - algorithm: two-tier dispatch, non-best-effort first
6. adaptive retry-after formula
   - `base + queue_depth * queue_step + inflight * inflight_step`
   - all constants crate-private, deterministic, test-fixed

스니펫:

```rust
let retry_after_ms =
    base_ms + (queue_depth as u64 * queue_step_ms) + (inflight as u64 * inflight_step_ms);
```

## Tests

1. FIFO preserves arrival order
2. weighted fair scheduling gives favored class more share without starvation
3. DRR rotates across classes deterministically
4. deadline-aware prioritizes tighter slack
5. best-effort class does not starve interactive class
6. fixed retry-after exact value
7. adaptive retry-after deterministic for same state

## Not Done If

1. fairness enum은 있는데 scheduling 차이가 없다.
2. retry-after가 hardcoded one-path only다.
3. best-effort가 query/search를 굶긴다.
