# BG25-002 — bounded strict ingress

- 상태: PLANNED
- 우선순위: P0
- 선행: BG25-001 D1–D3
- 소유: contract/ingress owner, host integration owner

## 목적

Raw DTO 호환성을 유지하면서 untrusted bytes를 execution/config authority로 승격하는 단일 strict ingress를 만든다. parsing 전 byte cap, decode 중 depth/stage cap, 모든 중첩 unknown/duplicate key, 명시적 wait와 blocking dispatch를 fail closed로 처리한다.

## 근거

- `TaskScope::Child.parent_awaits`는 `#[serde(default)]`다.
- `TaskSpec.stack_size_bytes`는 optional이며 unknown key는 무시된다.
- topology의 `physical_domains`도 default 가능하다.
- 현재 validator에는 stage-count와 preparse-byte 상한이 없다.

## 변경 파일

- 신규 후보 `crates/taskmesh/src/ingress.rs`
- `crates/taskmesh/src/{lib,builder,execution_plan}.rs`, `crates/taskmesh/Cargo.toml`, 필요 시 `Cargo.lock`
- fixture 후보 `crates/taskmesh/tests/{strict_ingress,strict_ingress_host}.rs`
- raw control: `crates/taskmesh-contract/tests/{contract_roundtrip,task_plan_validation}.rs`
- 공개 문서 두 파일

## 작업 계획

1. 실제 workload 근거로 byte/depth/stage 한계를 결정한다.
2. `serde_json::Value` 선파싱 없이 duplicate를 보존하는 strict visitor/mirror를 구현한다.
3. child는 `parent_operation_id`, `parent_stage`, `parent_awaits`를 모두 명시하게 한다.
4. blocking은 `shared_blocking` 또는 `requested_stack { stack_size_bytes }` tag를 요구한다.
5. strict config를 기존 topology/policy/Builder 검증으로 승격한다.

## DoD

- `BG25-002-D04`: bytes/depth/stage 0, 1, limit, limit+1이 정해진 단계에서 typed reject되고 과대 allocation 전에 멈춘다.
- `BG25-002-D21`: top-level/nested unknown·duplicate와 `physical_domans`가 strict config에서 거절되고 capacity side effect가 없다.
- `BG25-002-D22`: 나머지 parent identity가 유효한 child에서 `parent_awaits` 누락·오타만 정확히 거절한다.
- `BG25-002-D25`: stack key 누락·오타가 worker/permit/ticket 0으로 끝나며 명시적 shared control만 shared pool을 사용한다.
- `BG25-002-H33`: single-slot parent/child에서 raw false fallback과 strict reject, 명시된 cycle reject를 각각 관측한다.

## 검증

- Owner-local contract+host negative/control fixture.
- 실제 strict entrypoint가 default `just test`에 수집되는지 확인한다.
- 외부 deployment ingress가 별도면 그 consumer receipt 없이는 integration DoD를 닫지 않는다.

## 인계 및 중단 조건

- Raw `TaskSpec` Deserialize를 breaking 변경하려면 BG25-001로 되돌린다.
- strict parsing을 두 crate에 중복 구현하지 않는다.
