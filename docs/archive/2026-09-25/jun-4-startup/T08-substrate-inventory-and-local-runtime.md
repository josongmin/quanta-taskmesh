# T08 Substrate Inventory and Local Runtime

## Summary

substrate inventory를 repo-native SSOT로 만들고 `local_runtime` 예외를 통제한다.

## Decisions Frozen

1. built-in capability pool 목록은 고정이다.
2. substrate registration은 explicit다.
3. `local_runtime`는 catch-all escape hatch가 아니다.
4. allowlist/baseline-ratchet는 문서와 fixture로 같이 간다.

## Files To Touch

1. `crates/taskmesh-contract/src/lib.rs`
2. `crates/taskmesh-core/src/lib.rs`
3. `crates/taskmesh-tokio/src/lib.rs`
4. `README.md`
5. `crates/taskmesh-core/tests/substrate_inventory.rs` 신규
6. `crates/taskmesh-tokio/tests/local_runtime_guard.rs` 신규
7. `docs/taskmesh-library-spec.md`

## Implementation

1. built-in substrate records를 explicit canonical set로 고정한다.
2. inventory registration API를 추가한다.
   - duplicate detect
   - invalid capability reject
   - missing kind/capability metadata reject
3. snapshot이 substrate inventory를 실제로 노출하게 한다.
4. `run_local`은 `SubstrateHint::LocalRuntime` 또는 equivalent classified work에서만 허용되게 guard를 넣는다.
5. baseline-ratchet/allowlist 개념을 README와 spec에 간결히 추가한다.

구체 선택:

1. 자료구조
   - `BTreeMap<String, SubstrateRecord>` registry
2. fixture format
   - `crates/taskmesh-core/tests/fixtures/substrate_allowlist.json`
3. built-in canonical set
   - `cpu`
   - `blocking`
   - `large_stack`
   - `maintenance`
   - `local_runtime`
4. guard rule
   - first stage `substrate_hint`가 `LocalRuntime`가 아니면 `run_local` reject
5. registration rule
   - duplicate substrate name reject
   - missing capability pool reject except `AuthorityOnly`

스니펫:

```json
{
  "builtins": ["cpu", "blocking", "large_stack", "maintenance", "local_runtime"]
}
```

## Tests

1. built-in substrates appear in snapshot
2. duplicate substrate registration reject
3. invalid substrate record reject
4. `run_local` with non-local-runtime classified work reject
5. `run_local` with local-runtime classified work accept

## Not Done If

1. substrate inventory가 builder convenience 수준에 머문다.
2. `local_runtime`와 general async path 구분이 안 된다.
3. snapshot에서 substrate visibility가 없다.
