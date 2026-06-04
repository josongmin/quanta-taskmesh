# T01 Contract Freeze

## Summary

`taskmesh-contract` public wire/domain contract를 placeholder 없이 닫고, 문서와 코드의 canonical naming을 완전히 일치시킨다.

## Decisions Frozen

1. canonical public names:
   - `TaskSpec`
   - `TaskClass`
   - `TaskStage`
   - `SubstrateHint`
   - `AdmissionVerdict`
   - `GovernorError`
   - `RunError<E>`
   - `Snapshot`
   - `SubstrateRecord`
2. builder DSL은 convenience layer다.
3. freeze target은 data contract와 trait surface다.
4. public API는 product-neutral naming만 쓴다.

## Files To Touch

1. `crates/taskmesh-contract/src/lib.rs`
2. `docs/taskmesh-library-spec.md`
3. `docs/taskmesh-external-interface.md`
4. `README.md`
5. `crates/taskmesh-contract/tests/contract_roundtrip.rs` 신규
6. `crates/taskmesh-contract/tests/contract_builders.rs` 신규

## Implementation

1. 현재 contract 타입 전부를 canonical surface로 재검토한다.
   - `Snapshot`
   - `SubstrateRecord`
   - `PlanSource`
   - `ClassificationRationale`
   - `AdmissionVerdict`
   - `GovernorError`
   - `RunError<E>`
2. placeholder 의미만 있고 wire shape가 빈 타입이 남아 있으면 모두 concrete field/variant를 부여한다.
3. serde/clone/eq/hash/default 지원 범위를 타입별로 의도적으로 정리한다.
   - string-like identifier는 `Eq/Ord/Hash`
   - policy/config/value object는 `Clone/Eq`
   - runtime-only error는 serde 비대상 허용
4. docs 3종의 code snippet을 contract actual code에 맞춘다.
5. `TaskSpec::operation(...)`와 `child_of(...)`가 `root_operation_id`를 어떻게 채우는지 문서와 code comments 없이 코드 의미로 명확히 한다.

구체 선택:

1. 라이브러리
   - `serde`
   - `serde_json` (test only)
2. derive 기준
   - identifier/value object: `Debug + Clone + Eq + Ord + Hash + Serialize + Deserialize`
   - config/policy: `Debug + Clone + Eq + Serialize + Deserialize`
   - runtime error: serde 비대상 유지
3. canonical helper style
   - `new()` constructor
   - builder-like setter chaining
   - `Default`는 policy/config/value object에만 허용

스니펫:

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskClass(Cow<'static, str>);
```

## Tests

1. serde roundtrip:
   - `TaskSpec`
   - `ClassPolicy`
   - `RuntimeConfig`
   - `Snapshot`
2. builder smoke:
   - `TaskSpec::io/blocking/cpu`
   - `ClassPolicy::new()`
   - `ResourceBudget::new()`
   - `TopologyConfig::new()`
3. identifier semantics:
   - `TaskClass` and `TaskStage` ordering/hash/equality
4. docs compile:
   - public README/example snippet compile test
5. serde_json fixture roundtrip:
   - pretty JSON encode/decode for `TaskSpec` and `RuntimeConfig`

## Not Done If

1. public type 이름이 docs/code에서 다르다.
2. placeholder enum/struct가 남아 있다.
3. README/spec/interface 중 하나라도 다른 public contract를 설명한다.
4. builder convenience와 canonical freeze target 경계가 문서에 안 적혀 있다.
