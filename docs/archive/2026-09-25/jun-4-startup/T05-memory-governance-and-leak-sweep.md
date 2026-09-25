# T05 Memory Governance and Leak Sweep

## Summary

memory governance를 aspiration이 아니라 executable semantics로 만든다.

## Decisions Frozen

1. `Estimated`, `Measured`, `Hybrid` 모두 executable semantics를 가진다.
2. `bytes_per_unit`는 measured/hybrid의 필수 환산 primitive다.
3. leak-detecting은 이름만이 아니라 sweep path를 가진다.
4. degrade는 explicit fallback class가 필요하다.

## Files To Touch

1. `crates/taskmesh-contract/src/lib.rs`
2. `crates/taskmesh-core/src/lib.rs`
3. `crates/taskmesh-core/tests/memory_modes.rs` 신규
4. `crates/taskmesh-core/tests/memory_overcommit.rs` 신규
5. `crates/taskmesh-core/tests/leak_sweep.rs` 신규

## Implementation

1. `MemoryPermitMode`
   - `Estimated`: configured units 선점
   - `Measured`: measured byte input를 unit으로 환산
   - `Hybrid`: estimate로 선점 후 measured reconcile
2. `MemoryUnitScale`를 core에서 실제로 사용한다.
3. `MemoryReleasePolicy`
   - `OnTaskCompletion`
   - `OnStageBoundary`
   - `LeakDetecting`
4. `MemoryOvercommitPolicy`
   - `Reject`
   - `Queue`
   - `DegradeToLight { fallback_class }`
5. `Governor::reap_leaks(...)`를 no-op가 아니라 실제 sweep/report path로 바꾼다.
6. memory accounting state는 inflight permit state와 연결한다.
7. `LeakSweepReport`는 reclaimed permits / suspected leaks를 실제 값으로 채운다.

구체 선택:

1. 자료구조
   - `BTreeMap<PermitId, PermitLedger>`
2. ledger shape
   - `reserved_units`
   - `measured_bytes`
   - `effective_units`
   - `leased_at_ms`
   - `last_touched_ms`
   - `released`
3. unit conversion
   - `effective_units = ceil(measured_bytes / bytes_per_unit)`
4. hybrid reconcile
   - `effective_units = max(estimated_units, measured_units)`
5. leak sweep
   - startup 세트 기본값: `DEFAULT_LEAK_STALE_MS = 60_000`
   - `reap_leaks`는 `last_touched_ms + stale_after_ms < now_ms` 조건으로 stale permit 회수
6. stage boundary release
   - child/stage-local memory delta만 반환
   - root accounting은 유지

스니펫:

```rust
struct PermitLedger {
    reserved_units: u32,
    measured_bytes: u64,
    effective_units: u32,
    leased_at_ms: u64,
    last_touched_ms: u64,
    released: bool,
}
```

## Tests

1. measured/hybrid require meaningful `bytes_per_unit`
2. estimated mode uses configured permit cost
3. hybrid reconcile path updates accounting
4. overcommit reject path
5. overcommit queue path
6. overcommit degrade path uses fallback class
7. stage boundary early release returns memory units
8. leak sweep finds unreleased permit state
9. hybrid mode keeps `max(estimated, measured)` invariant

## Not Done If

1. leak-detecting이 이름만 있고 sweep path가 없다.
2. measured mode가 bytes-per-unit 없이 동작한다고 가정한다.
3. degrade path가 fallback class 없이 움직인다.
