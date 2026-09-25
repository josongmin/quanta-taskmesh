# S25-001 — 계약·신뢰 경계 고정

- 상태: PLANNED; 우선: P0; 선행: 없음; 소유: contract/API 통합 owner.
- 변경 권위: 공개 계약 문서와 DTO/API 결정. engine/host 소스는 각 후속 티켓의 owner가 적용.
- 근거: `crates/taskmesh-contract/src/{task,validation,config,ports}.rs`, `crates/taskmesh/src/{builder,runtime}.rs`, `crates/taskmesh-engine/src/shared/mod.rs`, [DECISIONS](DECISIONS.md).

## 목적

raw wire, validated plan, 배포 ingress, Governor direct API, Tokio host의 보장 범위가 다르다. 먼저 [D-S25-01–09](DECISIONS.md)를 실제 consumer surface와 비교해 확정한다. 결정마다 현재 동작, 목표 동작, 검증 주체, migration/semver, 타입/오류, 외부 통합 owner를 기록한다. stage/reduce 자동 실행, 임의 await 감지, shared executor의 ambient 작업 제한은 약속하지 않는다.

후속 티켓이 서로 다른 의미의 “안전한 입력”, “완료”, “소유 ID”를 구현하지 않게 계약을 잠근다. 이것은 **문서·공개 API 결정 티켓**이다. `DECISIONS.md`의 권고는 이미 구현된 보장이 아니며, 아래 각 항목에 현행/목표/실패 시 동작/호환성/consumer owner가 채워져야 승인된다. `RunFor`의 worker-start 예산, 자동 실행되지 않는 stage/reducer, 직접 Governor와 Tokio host의 예약 범위 차이는 이미 공개된 의미로 유지한다.

## 변경 파일

| 구분 | 경로 | 이 티켓에서 할 일 |
|---|---|---|
| 근거 | `crates/taskmesh-contract/src/{task,validation,config,topology,snapshot,verdict}.rs` | raw DTO/default/shape/version과 오류 어휘 확인 |
| 근거 | `crates/taskmesh-engine/src/{shared/mod,engine/governor,engine/state}.rs` | raw ID, close 전 preflight, parent generation 확인 |
| 근거 | `crates/taskmesh/src/{builder,runtime,execution_plan}.rs`, `crates/taskmesh/src/executor/{mod,tokio_exec}.rs` | executor descriptor·dispatch·deadline/custody 확인 |
| 근거 | `crates/taskmesh-rayon/src/lib.rs`, `Cargo.toml`, `crates/taskmesh-contract/Cargo.toml` | optional adapter·MSRV·새 dependency/semver 영향 확인 |
| 수정 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | 최종 의미·제약·예제를 같은 문장으로 반영 |
| 조건부 수정 | `docs/adr/0003-sep-16-hardening-contracts.md`, 새 결정 ADR | Accepted 계약을 변경해야 할 때만 변경 근거와 migration 명시. 기존 ADR을 조용히 재해석하지 않음 |
| 외부 소유 | 배포 JSON ingress, classifier, parent planner, consumer repository | 이 저장소에 실제 경로가 없으면 owner/버전/receipt 위치를 명시. 존재를 추정하지 않음 |

## 구현 순서

1. **입력 권위(D-S25-01–03).** 배포가 실제로 어느 bytes→DTO→validated plan/config→Builder 경로를 쓰는지 inventory한다. raw `TaskSpec`/`RuntimeConfig` Serde 호환은 별도 표시한다. strict API의 공개 위치(계약 crate 또는 공식 adapter), byte/depth/stage 예산과 측정 근거, unknown/duplicate key 정책, dispatch tag의 wire version/migration을 고정한다. `serde_json`을 production dependency로 추가한다면 `Cargo.lock`, MSRV, 기능 플래그 영향을 적는다.
2. **계획·ID 권위(D-S25-04–05).** cross-plan parent stage membership을 검증할 planner와 그 실제 호출 지점을 지정한다. direct raw `PermitId`/`Ticket`은 현재 per-Governor 숫자이고 foreign 충돌 가능함을 명시한다. 목표 opaque owner-bound handle, 숫자 telemetry ID 분리, deprecated/raw API 유예 및 version 경계를 결정한다.
3. **호스트 소유권(D-S25-06–08).** `RunFor`와 `CompleteBy` 각각의 response fence, worker/runtime teardown fence, 정상 결과의 release fence를 타임라인으로 적는다. Tokio 밖 poll 시 typed reject 여부와 custom/Rayon adapter별 context 조건을 확정한다. 공유 executor의 Taskmesh 관할 합산 상한은 명시적 공유 authority가 있을 때만 보장한다.
4. **wire 소비자(D-S25-09).** `Snapshot.schema_version`과 enum variant가 별개임을 고정한다. 알 수 없는 schema/variant 처리, 버전 협상 주체, serde decode 오류 노출 방식을 정한다.
5. 각 결정을 후속 S25-002~012의 acceptance와 연결하고, 문서가 공개 API보다 앞서 보장을 주장하지 않는지 재독한다.

## DoD

- [ ] `S25-001-A01` strict ingress의 입력 경로·byte/depth/stage 상한 소유자가 지정되고 raw Serde 직접 사용의 제한이 문서화된다.
- [ ] `S25-001-A02` H30/H34/B25/B28/B22의 현재 동작과 목표 계약이 각각 결정되며 호환성 범위가 명시된다.
- [ ] `S25-001-A03` 공개 API 변경이면 in-repo 및 외부 consumer 목록, MSRV/Rayon 영향, semver 조치가 기록된다.
- [ ] `S25-001-A04` `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md`, 필요하면 ADR에 같은 의미가 반영된다.

결정이 나기 전에는 소스상 실패 후보를 현행 보장으로 서술하거나 후속 티켓의 acceptance를 임의 완화하지 않는다. 산출물은 설계 결정이며 production PASS가 아니다.

### 결정별 판정과 인계

| 묶음 | 결정 문서에 반드시 적을 판정 | 후속 인계 |
|---|---|---|
| ingress | raw/strict 경로별 허용 payload, 예산 초과와 오타의 typed 오류, pre-parse byte cap 위치, 실제 배포 연결점 | S25-002·003, 외부 ingress owner |
| identity | `(root, operation)`과 parent permit generation의 의미, parent plan membership의 owner, raw/opaque handle의 foreign 의미와 semver | S25-004, planner/consumer owner |
| execution | `RunFor`/`CompleteBy`의 기준 시각·응답·custody·drain 시점, Tokio 밖 poll의 오류, shared executor의 물리 상한 범위 | S25-005·006·007·011 |
| wire | Snapshot 버전 거부와 미래 enum variant decode 오류, old alias 유지 범위 | S25-003·004·012 |

**종료 조건:** D-S25-01~09 각각에 결정 상태, source anchor, 반례, API/오류 형태, migration, 담당 consumer, 적용 문서가 들어가고 후속 owner가 수락한다. 미정 항목은 “결정 완료”로 표시하지 않는다.

## 계획된 검증

`python3 docs/archive/2026-09-25/sep-25-engine-coverage/tickets/validate_plan.py`는 문서 연결 무결성만 확인하는 owner-local 명령이며 product 테스트가 아니다. 공개 계약을 실제로 바꾸는 후속 패치에서는 `just doctest`, `just rustdoc`, `just consumer-msrv`를 S25-012에 넘긴다. 문서 링크·실제 타입명·현재 source anchor를 재확인한다. 이 티켓에서는 production 테스트나 CI를 수행했다고 표시하지 않는다.

## 인계·중단 조건

배포 ingress/외부 planner/consumer가 저장소 밖이고 실제 소유자를 특정할 수 없으면 그 통합 DoD를 OPEN으로 남긴다. 목표 계약이 raw Serde·0.3 공개 API 또는 Accepted ADR과 충돌하면 구현 전에 migration/버전 결정을 갱신한다. 이 티켓 완료만으로 H30/H34 등의 결함이 해결됐다고 선언하지 않는다.
