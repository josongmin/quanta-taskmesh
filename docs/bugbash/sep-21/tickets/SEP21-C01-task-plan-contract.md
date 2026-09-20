# SEP21-C01 — Task plan contract normalization

- 상태: PLANNED
- 우선순위: P1 contract foundation
- 포함 finding: TM21-010
- 선행: 없음
- write lane: `contract-task`
- 후속 소비자: E04, H01, R01

## 목적

`TaskSpec`을 raw serialized data가 아니라 admission 전에 한 번 검증되는 product-neutral
governance plan으로 만든다. stage identity와 parent lineage의 의미를 contract가 소유하고,
engine/host가 각자 유사 validation을 재구현하지 않게 한다.

## RCA

- `PlanSource`가 provenance가 아니라 제품 기능 taxonomy를 public enum으로 노출한다.
- stage shape 검사가 engine의 `validate_shape`와 `reduce::validate_spec`에 분산되어 있고
  duplicate identity owner가 없다.
- child wait identity가 `(root, parent_stage)`에 머물러 sibling과 immediate parent를 구분할
  수 없다. E04가 정확한 wait graph를 만들 contract 정보가 부족하다.

## 확정 근거

- product-specific `PlanSource`: `crates/taskmesh-contract/src/task.rs:52-61`.
- stages가 governance authority라는 public contract: `crates/taskmesh-contract/src/task.rs:102-125`.
- duplicate를 보지 않는 shape validator: `crates/taskmesh-engine/src/features/composite/mod.rs:16-38`.
- per-entry reduce만 보는 validator: `crates/taskmesh-engine/src/features/composite/reduce.rs:17-35`.
- awaited-child 계약과 현재 scope shape: `crates/taskmesh-contract/src/task.rs:206-216`.

## 목표 계약

- `PlanSource`는 기존 JSON string wire를 보존하는 bounded validated opaque newtype으로
  교체한다. core는 `internal` 같은 최소 generic associated constant만 canonical하게 둔다.
- Search/Index/SDK 같은 기존 값은 한 migration window 동안 deprecated associated constant로
  decode/encode 호환을 유지하되, 새 제품 mapping은 consumer adapter가 소유한다.
- `TaskSpec::validate()` 또는 동등한 contract-owned validator가 zero stage, class mismatch,
  duplicate stage, incomplete reduce, invalid identifier를 한 번에 판정한다.
- awaited child는 `root_operation_id`, `parent_operation_id`, `parent_stage`를 명시한다.
  engine은 동일 root에서 active parent operation이 하나인지 검증한다. caller 문자열만 믿고
  capacity holder로 간주하지 않는다.
- validated result는 raw `TaskSpec`과 구분되는 `ValidatedTaskPlan` 또는 sealed token이다.

## 작업 플랜

1. `crates/taskmesh-contract/src/task.rs`
   - product-neutral bounded provenance newtype, manual serde와 migration constant를 추가한다.
   - `TaskScope::Child`에 immediate-parent identity를 추가한다.
   - stage/operation identity의 trim, empty, uniqueness 규칙을 정의한다.
2. 필요 시 `crates/taskmesh-contract/src/validation.rs`를 만들고 모든 plan validation을
   이동한다. `task.rs` builder와 serde path가 같은 validator를 사용한다.
3. `crates/taskmesh-contract/src/validation.rs`, `crates/taskmesh-contract/src/lib.rs`
   - typed `TaskPlanError`와 contract-crate export를 정리한다. executor/error domain인
     `verdict.rs`는 수정하지 않는다.
4. ticket closure evidence에 old `PlanSource`와 `awaited_child_of` migration의 before/after,
   wire/semver 영향, deprecated window를 구조화한다. shared README/CHANGELOG/spec 반영은 R01이
   모든 API delta를 한 번에 통합한다.
   - `taskmesh` facade re-export와 facade consumer migration은 H03이 final executor/facade API와
     함께 한 번만 통합한다.
5. engine admission wiring과 duplicate-plan state invariant는 E04가 final pending resolver와 함께
   소유한다. C01은 engine 파일을 직접 수정하지 않는다.

## 테스트 플랜

- `crates/taskmesh-contract/tests/contract_builders.rs`
  - product-neutral provenance builder, empty/invalid key, parent identity builder.
- `crates/taskmesh-contract/tests/contract_roundtrip.rs`
  - old wire migration 또는 명시적 breaking rejection, new wire roundtrip.
- contract test는 duplicate same/conflicting descriptor와 unknown/ambiguous parent shape를
  판정한다.
- engine state non-mutation과 sibling-parent spoof integration은 E04 acceptance다.
- contract-crate compile fixture에서 deprecated type 없이 새 API를 compile한다. `taskmesh`
  facade/MSRV fixture는 H01 acceptance로 넘긴다.

## DoD

- `SEP21-C01-A01`: contract validator가 malformed plan을 deterministic typed error로 판정한다.
- `SEP21-C01-A02`: 동일 stage name은 first/last-wins나 silent dedupe 없이 error다.
- `SEP21-C01-A03`: 같은 root의 active operation identity 중복과 parent identity shape를
  fail-closed 판정할 contract 정보가 존재한다.
- `SEP21-C01-A04`: core enum variant와 branching logic에 Search/Index/SDK 제품 taxonomy가 남지 않는다.
- `SEP21-C01-A05`: old JSON provenance 문자열은 decode/re-encode되며 Rust exhaustive enum
  match/`Copy` break는 R01 input manifest에 구조화된다.
- `SEP21-C01-A06`: E04가 raw string scan 없이 immediate parent permit을 결정할 충분한
  validated identity를 받는다.
- `SEP21-C01-A07`: semver/serde breaking change와 migration은 R01 input manifest에 등록된다.

## 금지되는 임시방편

- engine과 contract 양쪽에 validation 복제.
- old/new provenance를 무기한 dual-write.
- duplicate stage를 정렬 또는 dedupe해 입력 오류를 숨김.
- 모든 child permit을 parent capacity로 간주하는 lineage 우회.
- product-specific string을 opaque key 기본값으로 다시 박아 넣기.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh-contract
cargo test --locked -p taskmesh-engine --test reduce_policy_validation --test composite_root_attribution
```
