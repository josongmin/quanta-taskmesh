# T03 Core Admission Queue Permit

## Summary

`taskmesh-core`에 실제 bounded admission, queue, permit lifecycle을 넣는다. 현재 inflight-only skeleton을 governed execution state machine으로 올린다.

## Decisions Frozen

1. class lookup은 fail-closed다.
2. queue는 bounded다.
3. permit acquire/release lifecycle은 inflight accounting과 분리되지 않는다.
4. root operation id가 admission key 기준이다.

## Files To Touch

1. `crates/taskmesh-core/src/lib.rs`
2. `crates/taskmesh-core/tests/admission_queue.rs` 신규
3. `crates/taskmesh-core/tests/permit_lifecycle.rs` 신규

## Implementation

1. state를 inflight-only에서 queue-aware state로 확장한다.
   - queued count
   - pending requests
   - permit ownership
2. class not found 시 `AdmissionVerdict::UnknownClass` 즉시 반환.
3. class disabled path를 actual verdict로 구현.
4. `max_inflight` 도달 시:
   - overflow policy가 queueable이면 queue
   - 아니면 reject
5. `max_queue_depth` 초과 시 `QueueFull`.
6. release 시:
   - inflight 감소
   - queue에서 다음 작업 승격
7. `RequestKey`를 root operation 기반으로 다루되, same-key dedupe는 이번 티켓 범위에서 하지 않는다.
8. snapshot에 `queued`가 실제 값으로 반영되게 한다.

구체 선택:

1. 자료구조
   - `BTreeMap<TaskClass, ClassState>`
   - `VecDeque<PendingRequest>` per class
   - `BTreeMap<PermitId, PermitRecord>`
2. 새 core-private 타입
   - `ClassState { inflight, queued, queue }`
   - `PendingRequest { seq_no, request_key, root_operation_id }`
   - `PermitRecord { permit_id, class, root_operation_id }`
3. 순서 보장
   - 동일 class 내 FIFO
   - cross-class ordering은 T04 fairness에서 결정
4. `AdmissionDecision` 확장
   - `Admitted { permit_id }`
   - `Queued { queue_ticket }`
   - `Rejected(verdict)`

스니펫:

```rust
struct ClassState {
    inflight: u32,
    queued: u32,
    queue: VecDeque<PendingRequest>,
}
```

## Tests

1. unknown class -> `UnknownClass`
2. class disabled -> `ClassDisabled`
3. inflight below cap -> admitted
4. inflight at cap + queueable -> queue
5. inflight at cap + non-queueable -> reject
6. queue depth exceeded -> `QueueFull`
7. release promotes queued work
8. snapshot shows inflight/queued correctly
9. queued decision returns stable ticket ordering

## Not Done If

1. queue가 없다.
2. permit lifecycle이 inflight accounting과 분리된다.
3. queued work가 release 후 승격되지 않는다.
