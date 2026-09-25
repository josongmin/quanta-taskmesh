# T02 Config Builder and Validation

## Summary

topology/class policy/resource budget builder와 validation rule을 configuration construction 단계에서 fail-closed로 확정한다.

## Decisions Frozen

1. invalid policy는 runtime execution 중간이 아니라 construction 단계에서 reject한다.
2. `permit_cost > per_request_max_*` 또는 global max 초과는 hard failure다.
3. `Measured`/`Hybrid` memory mode는 `memory_unit_scale.bytes_per_unit > 0` 전제다.
4. `DegradeToLight`는 `fallback_class`가 필수다.

## Files To Touch

1. `crates/taskmesh-contract/src/lib.rs`
2. `crates/taskmesh-core/src/lib.rs`
3. `crates/taskmesh-contract/tests/config_builders.rs` 신규
4. `crates/taskmesh-core/tests/config_validation.rs` 신규

## Implementation

1. `TopologyConfig`, `CpuPoolConfig`, `ResourceBudget`, `ClassPolicy` builder API를 점검한다.
   - builder names는 유지
   - missing setter가 있으면 추가
2. `Governor::validate_policy(...)`를 actual configuration validator로 키운다.
3. 아래를 모두 validate한다.
   - per-request CPU limit overflow
   - per-request memory limit overflow
   - global CPU limit overflow
   - global memory limit overflow
   - `Measured`/`Hybrid` with invalid `bytes_per_unit`
   - `DegradeToLight` fallback class misuse
   - zero/meaningless queue settings if class is queueable
4. validation error는 `GovernorError::PolicyViolation(...)`로 통일한다.
5. unknown class reject는 runtime admit path semantics로 유지하되, config-side default-admit escape hatch는 열지 않는다.

구체 선택:

1. 라이브러리
   - 새 runtime dependency 추가 없음
   - test only `serde_json`
2. auto parallelism
   - `std::thread::available_parallelism()` 사용
   - `num_cpus` 추가 금지
3. validation owner
   - `taskmesh-core::Governor::validate_policy(...)`
   - contract crate는 pure data only
4. queueability rule
   - `OverflowPolicy::QueueWithinDepth`인 class는 `max_queue_depth > 0` 필수
5. memory scaling rule
   - `Measured`/`Hybrid`에서 `bytes_per_unit == 0`이면 reject

스니펫:

```rust
if matches!(class_policy.memory_permit_mode, MemoryPermitMode::Measured | MemoryPermitMode::Hybrid)
    && policy.resources.memory_unit_scale.bytes_per_unit == 0
{
    return Err(GovernorError::PolicyViolation("bytes_per_unit must be > 0".into()));
}
```

## Tests

1. valid minimal config passes
2. class CPU over budget fails
3. class memory over budget fails
4. invalid memory scaling fails
5. degrade policy without meaningful fallback shape cannot be constructed
6. builder smoke for topology/resource/class objects

## Not Done If

1. build 시점에 막아야 할 정책 오류가 runtime 중간까지 넘어간다.
2. validation rule이 contract 문서와 다르다.
3. invalid memory scaling이 허용된다.
