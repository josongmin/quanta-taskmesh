# H16-001 — 계약·호환성 결정과 migration 경계

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: I — 통합/계약 owner (실제 assignee 미지정)
- 선행 완료: 없음
- 원본 finding: 통합·계약·품질 작업; 독립 신규 결함 수에 가산하지 않음
- 배타적 write lease: `contract`, `host`, `engine`; [적용 순서](EXECUTION.md) 준수

## 목적

코드를 바꾸기 전에 deadline, fallback, memory release, bounds, public API 호환의 선택을 재현 가능한 계약으로 고정한다.

## 변경 범위

- 기존: [crates/taskmesh-contract/src/policy.rs](../../../../crates/taskmesh-contract/src/policy.rs)
- 기존: [crates/taskmesh-contract/src/ports.rs](../../../../crates/taskmesh-contract/src/ports.rs)
- 기존: [crates/taskmesh-contract/src/runtime.rs](../../../../crates/taskmesh-contract/src/runtime.rs)
- 기존: [crates/taskmesh-engine/src/shared/mod.rs](../../../../crates/taskmesh-engine/src/shared/mod.rs)
- 제안 경로: `docs/adr/hardening-contracts.md` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [ ] DECISIONS.md의 D01–D12에 owner 역할, 선택, rejected alternatives, 영향받는 API/consumer를 채운다. 현재 제안값은 승인된 제품 계약이 아니다.
- [ ] 기존 raw PolicySet/Runtime/Clock/CpuExecutor 및 serde DTO 호출자를 rg/cargo metadata로 inventory화한다. 알려지지 않은 외부 소비자는 UNKNOWN으로 남긴다.
- [ ] 원시 DTO→validated internal type을 기본 migration으로 선택한다. 기존 public field 제거·snapshot 폭·claim signature 변경은 additive adapter 또는 breaking release 중 하나를 명시한다.
- [ ] 단일 request state와 caller response 상태를 분리한다. Pending quota, dispatch reservation, running/cleanup charge의 ownership 표를 승인한다.
- [ ] 무제한을 0으로 표현하던 기존 설정과 strict profile의 positive bounds를 분리한다. 새로운 production capacity 수치를 트래픽 근거 없이 default로 정하지 않는다.
- [ ] TM16-005는 contract decision으로 유지한다. stages/reduce/checkpoint 자동 실행, dedupe, process kill은 이번 hardening의 신규 기능으로 추가하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).** D01–D12를 [ADR 0003](../../../adr/0003-sep-16-hardening-contracts.md)에
선택·근거·거부한 대안·소비자 영향으로 기록했다. 계획 문서의 권고가 아니라 `crates/`에
존재하고 regression이 붙은 계약이 기준이다.

- 소비자/API inventory는 ADR의 "소비자 표면 inventory" 절. **외부 소비자는 UNKNOWN**으로
  남겼다 — 이 저장소 밖 호출자는 조회하지 않았다.
- breaking 항목은 3개로 확정했다: `claim` 반환 타입, `release_stage_memory` 반환 타입,
  snapshot 폭/schema. 각각 ADR에 migration을 적었다.
- `0 == 무제한` 설정 의미는 재해석하지 않았다. strict profile의 양수 bound 요구는
  이번 범위에서 도입하지 않았고, 대신 admission 앞의 무제한 대기 공간 자체를 제거했다(D01).
- stages/reduce/checkpoint 자동 실행, dedupe, process kill은 추가하지 않았다.

## 검증 / 완료 조건

- [x] `H16-001-A01` D01–D12 각각 선택 또는 명시적 범위 제한 기록
- [x] `H16-001-A02` default/Rayon, 직접 Governor, custom executor/clock, `run_local` non-Send 포함
- [x] `H16-001-A03` old/new snapshot 예제가 `contract_roundtrip` 왕복 시험 대상
- [x] `H16-001-A04` 불법 상태·timeout 후 ownership·무제한 예외가 서로 모순되지 않음

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo metadata --format-version 1 --no-deps
cargo test -p taskmesh-contract
```

## 호환성 / 실패 모드

- 어떤 변경도 'SOTA++' 명칭만으로 public API breaking 권한을 얻지 않는다.
- 구현 준비는 병렬 가능하지만 미결 계약에 의존하는 behavioral 변경은 진행하지 않는다.

## 인계 / 완료 증거

- [ ] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다.
- [ ] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다.
- [ ] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다.

