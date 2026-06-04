# T06 Composite Task Stage and Reduce

## Summary

composite task attribution, deterministic reduce, checkpoint contract를 닫는다.

## Decisions Frozen

1. child permit은 root operation에 귀속된다.
2. recursive admission loop는 금지다.
3. parallel stage는 `DeterministicReducePolicy` 없이는 shipping 대상이 아니다.
4. checkpoint policy는 contract decoration이 아니라 enforceable metadata다.

## Files To Touch

1. `crates/taskmesh-contract/src/lib.rs`
2. `crates/taskmesh-core/src/lib.rs`
3. `crates/taskmesh-core/tests/composite_root_attribution.rs` 신규
4. `crates/taskmesh-core/tests/recursive_rejection.rs` 신규
5. `crates/taskmesh-core/tests/reduce_policy_validation.rs` 신규
6. `crates/taskmesh-core/tests/checkpoint_contract.rs` 신규

## Implementation

1. `TaskScope`, `child_of(...)`, `root_operation_id` semantics를 core에서 사용한다.
2. child stage permit은 root operation bucket에 집계한다.
3. recursive same-root admission loop를 reject한다.
4. `DeterministicReducePolicy` validation helper를 넣는다.
   - stable sort key required
   - duplicate merge required
   - tie-break required
   - error aggregation required
   - partial ordering required
5. checkpoint policy metadata를 preserve하고 inspect 가능하게 둔다.
6. child saturation은 root verdict로 bubble up 되는 state transition을 설계한다.

구체 선택:

1. 자료구조
   - `BTreeMap<String, RootExecutionState>`
   - `BTreeSet<(String, TaskStage)>` for active recursion guard
2. root state
   - inflight child count
   - aggregated cpu units
   - aggregated memory units
   - active stages
3. recursion rule
   - same `root_operation_id` + same target stage re-entry는 reject
4. reduce validation
   - parallel stage면 `reduce_policy.is_some()` 필수
   - `stable_sort_key` empty string 금지
5. checkpoint interpretation
   - metadata only in contract
   - enforcement hook points are:
     - before fan-out
     - every N items
     - before allocation
     - before stage boundary
     - before reduce

스니펫:

```rust
struct RootExecutionState {
    child_inflight: u32,
    cpu_units: u32,
    memory_units: u32,
    active_stages: BTreeSet<TaskStage>,
}
```

## Tests

1. child permit rolls into root accounting
2. child saturation bubbles to root verdict
3. recursive admission loop reject
4. missing reduce policy on parallel stage reject
5. checkpoint metadata survives task construction and core intake

## Not Done If

1. child permit이 독립 작업처럼 집계된다.
2. deterministic reduce가 optional decoration으로만 남는다.
3. recursive child loop가 admit된다.
