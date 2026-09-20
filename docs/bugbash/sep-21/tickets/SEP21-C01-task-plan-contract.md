# SEP21-C01 — Task plan contract normalization

- 상태: LOCALLY_VERIFIED
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

## Closure evidence (2026-09-21)

Focused source:

- branch/HEAD: `main@3da26491997c81435c969bd05b8a20438e360f99`
- HEAD tree: `65a9061dcae7c815ab99477f0d98eabec1fcba8e`
- C01 source/test dirty digest: `sha256:11553d63d041be8ad6a13167edfc2d5649e035db98bbba37f5c4e12ce2e4dc7b`
  (binary diff of the four tracked contract source/test paths plus Git blob identities of the two
  new files; this deliberately excludes this self-referential ticket and other owners' dirty paths)
- new validation artifact digests:
  - `validation.rs`: `sha256:6261e153bd94d3c65f3d95d9b7dc2e229a23e410cc5d9978535f47f7908d7b1d`
  - `task_plan_validation.rs`: `sha256:fe80f9bdfc6b6979205e589bdf3a46cd62763d9ae6a2f7314aa67cf682534c1b`

Changed paths:

- `crates/taskmesh-contract/src/lib.rs`
- `crates/taskmesh-contract/src/task.rs`
- `crates/taskmesh-contract/src/validation.rs`
- `crates/taskmesh-contract/tests/contract_builders.rs`
- `crates/taskmesh-contract/tests/contract_roundtrip.rs`
- `crates/taskmesh-contract/tests/task_plan_validation.rs`
- `docs/bugbash/sep-21/tickets/SEP21-C01-task-plan-contract.md`

This is contract evidence only; engine admission integration, facade/MSRV migration, workspace-wide
tests, semver proof, hosted CI, and release qualification were not run here and remain assigned to
E04/H01/H03/R01.

| Acceptance | Evidence |
| --- | --- |
| SEP21-C01-A01 | `TaskSpec::validate`, `TryFrom<TaskSpec> for ValidatedTaskPlan`, and validated-plan serde all use the single validator in `validation.rs`; typed negative fixtures cover zero stages, identifiers, class mismatch, reduce shape, and lineage. |
| SEP21-C01-A02 | `DuplicateStage` rejects an identical repeated descriptor; `ConflictingStage` rejects the same stage identity with a different descriptor. No sorting or dedupe occurs. |
| SEP21-C01-A03 | `TaskScope::Child` now carries required `parent_operation_id` in addition to root and parent stage. Empty, child-equal-parent, and child-equal-root identities reject. Active-operation uniqueness remains E04 state-owner work. |
| SEP21-C01-A04 | `PlanSource` is an opaque bounded string newtype. Product names remain only as deprecated 0.2 migration constants; there is no product enum or core branching. |
| SEP21-C01-A05 | All six legacy provenance JSON strings decode and re-encode byte-for-byte. The former enum's `Copy` and exhaustive-match source compatibility intentionally breaks. |
| SEP21-C01-A06 | A validated child plan exposes `root_operation_id`, exact `parent_operation_id`, `parent_stage`, and `parent_awaits`, sufficient for E04 to resolve the active immediate-parent permit without raw string scans. |
| SEP21-C01-A07 | The R01 input manifest below records the Rust and serde breaks and the bounded 0.2 provenance migration window. |

Validation evidence:

```text
CARGO_TARGET_DIR=target/sep21/contract-c01 cargo test --locked -p taskmesh-contract
exit 0; 56 passed, 0 failed (11 task-plan adversarial tests)

uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
exit 0

git diff --check
exit 0
```

### R01 input manifest

| Surface | Before | After / compatibility |
| --- | --- | --- |
| Provenance Rust API | `Copy` product enum with exhaustive variants | Opaque non-`Copy` `PlanSource`; `new` validates a maximum 128-byte canonical key. Legacy associated constants are deprecated for the 0.2 migration window and are removed at the next breaking release. |
| Provenance wire | Enum strings such as `"SearchAdapter"` and `"Internal"` | Every legacy string decodes and re-encodes unchanged. New internal plans emit canonical `"internal"`. Invalid/empty/oversize strings reject during decode. |
| Child Rust API | `child_of(root, stage)` / `awaited_child_of(root, stage)` | Both builders require `(root, immediate_parent_operation, parent_stage)`. This is an intentional source break. |
| Child wire | `parent_stage` plus optional `parent_awaits` | `parent_operation_id` is required. Legacy child payloads missing it reject as ambiguous; root payloads are unchanged. |
| Admission boundary | Public raw `TaskSpec` only | `ValidatedTaskPlan` is sealed and constructible only through validation. Raw `TaskSpec` remains the serde/builder input type. Engine admission wiring is E04-owned. |
