# H16-004 — 정확한 resource accounting과 snapshot 폭

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-003](H16-003-terminal-ticket-lifecycle.md)
- 원본 finding: [TM16-008](../../../../bugbash/sep-16-general/tickets/TM16-008-resource-counter-saturation.md)
- 배타적 write lease: `contract`, `engine`; [적용 순서](EXECUTION.md) 준수

## 목적

capacity check와 실제 ledger를 같은 정확한 수학으로 계산하고 saturation을 오류 은폐 수단에서 제거한다.

## 변경 범위

- 기존: [crates/taskmesh-contract/src/resource.rs](../../../../../crates/taskmesh-contract/src/resource.rs)
- 기존: [crates/taskmesh-contract/src/snapshot.rs](../../../../../crates/taskmesh-contract/src/snapshot.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh-engine/src/shared/mod.rs](../../../../../crates/taskmesh-engine/src/shared/mod.rs)
- 기존: [crates/taskmesh-engine/src/features/memory/mod.rs](../../../../../crates/taskmesh-engine/src/features/memory/mod.rs)
- 제안 경로: `crates/taskmesh-engine/tests/hardening_exact_accounting.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] 원시 measured bytes와 reservation units, aggregate units를 분리한다. 권장 내부 폭은 per-request u64/aggregate checked u128이며 D06에서 확정한다.
  → per-request `u32` / aggregate `u128`로 확정(D06)
- [x] grant/reconcile/release/root/class/global 합계를 공통 delta 함수로 바꾼다. capacity comparison 전에 widening하고 실패 시 부분 commit을 하지 않는다.
- [x] measured overage는 현실 사용량으로 기록하고 신규 admission을 차단한다. 합계가 표현되지 않는 경우 0/이전값 유지로 정상인 척하지 않고 accounting fault 상태로 fail-closed한다.
- [x] assert_consistent와 독립 oracle가 동일 saturation helper를 재사용하지 않도록 한다. pending/running/root projected sums와 reserve/active 구분을 검증한다.
- [x] SnapshotV2 또는 checked legacy conversion을 정의한다. u128을 JSON double로 내보내지 않고 decimal-string 등 정확한 wire representation을 명시한다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- per-request cost는 `u32`, class/global/root aggregate는 `u128`이며 capacity 비교는
  전부 checked다. 포화 연산은 capacity 경로에서 제거했다.
- `MemoryUnitScale::units_for`가 `Result<u32, ResourceConversionError>`를 반환한다.
  변환 실패는 ledger를 부분 갱신하지 않는다.
- snapshot의 held 값은 `u128`이며 decimal **string**으로 직렬화한다(JSON number는
  `f64`를 거쳐 값이 달라진다). `SNAPSHOT_SCHEMA_VERSION = 2`.
- `Governor::permit_ledgers()`로 독립 oracle이 aggregate를 처음부터 재계산할 수 있다.
  conservation 검사는 검사 대상과 산술을 공유하지 않는다.
- `CapacityBlock::AccountingFault`는 fail-closed 분기다. 현재 폭에서는 도달 불가이며
  ADR에 그렇게 적었다 — 도달 가능한 척하지 않는다.

Regression: `crates/taskmesh-engine/tests/hardening_exact_accounting.rs` (6),
`crates/taskmesh-contract/tests/contract_roundtrip.rs`, `contract_builders.rs`.

## 검증 / 완료 조건

- [x] `H16-004-A01` CPU/memory MAX-1/MAX/2^31×2에서 초과 요청 reject
- [x] `H16-004-A02` 큰 값·다중 class·release 순열 합계가 독립 oracle과 일치
- [x] `H16-004-A03` 오류·unsupported 변환이 state를 부분 갱신하지 않음
- [x] `H16-004-A04` debug invariant도 wrap/saturate하지 않음 (`u128` 재계산)
- [x] `H16-004-A05` snapshot schema 이동은 명시적 버전과 왕복 test 보유

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test prop_invariants --test memory_modes --test memory_overcommit
cargo test -p taskmesh-contract --test contract_roundtrip
```

## 호환성 / 실패 모드

- 폭 확장만으로 MemoryUnitScale의 개별 값 saturation까지 자동 해결되지 않는다.
- 메모리 soft accounting을 physical allocator hard cap으로 표현하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

