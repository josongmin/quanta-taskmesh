# T07 Tokio Runtime Adapter

## Summary

`taskmesh` facade를 실제 usable runtime adapter로 만든다.

## Decisions Frozen

1. governor rejection과 task failure는 타입 레벨에서 분리된다.
2. `run_local`은 `local_runtime` 예외 전용이다.
3. timeout/cancel은 adapter layer concern이다.
4. join failure는 task failure가 아니라 governor/runtime side failure다.

## Files To Touch

1. `crates/taskmesh-tokio/src/lib.rs`
2. `crates/taskmesh-tokio/tests/runtime_io.rs` 신규
3. `crates/taskmesh-tokio/tests/runtime_blocking.rs` 신규
4. `crates/taskmesh-tokio/tests/runtime_cpu.rs` 신규
5. `crates/taskmesh-tokio/tests/runtime_local.rs` 신규
6. `crates/taskmesh-tokio/tests/runtime_cancel_timeout.rs` 신규

## Implementation

1. `run_io`, `run_blocking`, `run_cpu`, `run_local` 공통 admit/release path를 helper로 묶는다.
2. cancellation/deadline adapter를 추가한다.
   - pre-submit cancel
   - bounded wait
   - timed acquire
3. `spawn_blocking` join failure mapping을 정리한다.
4. `run_local`은 non-`Send` future도 수용 가능한 current-thread/local-set compatible path로 정리한다.
5. `Runtime::snapshot()`은 그대로 core snapshot을 노출한다.
6. `config()` accessor는 runtime config inspection용으로 유지한다.

구체 선택:

1. 라이브러리
   - `tokio`
   - `tokio-util::sync::CancellationToken`
2. primitives
   - `tokio::time::timeout`
   - `tokio::task::spawn_blocking`
   - `tokio::task::LocalSet`
   - `tokio::sync::oneshot`
3. cancel/deadline path
   - queued wait는 `timeout(...)`으로 bounded wait
   - cancel token set이면 pre-submit reject
4. `run_local`
   - `LocalSet::run_until(...)` 기반
   - non-`Send` future 허용
5. join failure mapping
   - `JoinError` -> `GovernorError::PolicyViolation(...)`
6. repeated admit/release code
   - private helper `with_permit(...)`

스니펫:

```rust
let out = tokio::time::timeout(deadline, wait_for_permit).await;
```

## Tests

1. `run_io` happy path
2. `run_blocking` happy path
3. `run_cpu` happy path
4. `run_local` with non-`Send` future
5. cancel before submit -> governor-side rejection
6. timeout acquire -> typed governor error
7. task error remains `RunError::Task`
8. join failure remains governor/runtime-side error
9. `CancellationToken` fires before submit -> typed governor-side rejection

## Not Done If

1. `local_runtime` capability가 public API에서 unreachable이다.
2. runtime error와 task error가 다시 flatten된다.
3. timeout/cancel contract가 façade에서 안 보인다.
