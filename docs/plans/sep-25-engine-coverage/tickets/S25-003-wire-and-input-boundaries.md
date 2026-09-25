# S25-003 — wire·입력·consumer 오류 의미

- 상태: PLANNED; 우선: P1; 선행: [S25-001](S25-001-contract-boundaries.md); 소유: contract owner, host/consumer owner.
- 주 담당 시나리오: B18, B24, D03, D13, D14, D17, D23.
- 근거: `crates/taskmesh-contract/src/{task,validation,snapshot,verdict}.rs`, `crates/taskmesh-engine/src/engine/governor.rs`, `crates/taskmesh/src/runtime.rs` 및 기존 `contract_roundtrip.rs`, `snapshot_oracle.rs`.

## 목적

wire 버전, 식별자, provenance, close 오류 우선순위를 각각 독립 계약으로 다룬다. 공개 소비자가 사용하는 decode/validate/admit 순서를 fixture에 명시한다. 현재 `docs/taskmesh-library-spec.md`는 NUL/비ASCII class가 실행된다고 적지만 validation 경로는 사전 거절하므로 D14에서 문서를 소스에 맞춘다. metadata의 사실 여부는 engine이 알 수 없으므로 외부 classifier의 생성·검증 책임으로 둔다. derived enum Deserialize의 미래 variant 오류는 caller가 처리할 수 있는 error path로 문서화한다.

`PlanSource`는 문자열 Serde와 `PlanSource::new`의 동일 identifier 검사, `ClassificationRationale`은 enum이다. `Snapshot`은 v2 필드를 포함하지만 derived Deserialize가 버전을 검증하지 않는다. `#[non_exhaustive]`는 Rust 패턴 매칭 규칙이지 미래 variant 수용 decoder가 아니다. 이 구별을 공개 consumer 경계에서 검증한다.

## 변경 파일

| 구분 | 경로 | 작업 |
|---|---|---|
| 기존 근거·조건부 수정 | `crates/taskmesh-contract/src/{task,validation,snapshot,verdict,config,topology}.rs` | wire spelling, byte-offset 오류, snapshot/version, enum decode 확인. 의미 변경은 재현된 결함과 S25-001 결정이 있을 때만 |
| 기존 근거·조건부 수정 | `crates/taskmesh-engine/src/engine/governor.rs`, `crates/taskmesh/src/runtime.rs` | `close_admission` 이전 preflight 순서와 requested-stack thread label 확인; rustdoc 불일치 정정 |
| 기존 fixture | `crates/taskmesh-contract/tests/{contract_roundtrip,task_plan_validation,snapshot_oracle,error_surface}.rs` | B18/D03/D13/D23의 wire·오류 control 보강 |
| 기존 fixture | `crates/taskmesh-engine/tests/{provenance_preservation,hardening_close_admission}.rs`, `crates/taskmesh/tests/hardening_executor_protocol.rs` | B24/D17/D14의 engine·OS side effect 판정 |
| 신규 fixture 제안 | `crates/taskmesh-contract/tests/wire_consumer_negatives.rs`, `crates/taskmesh/tests/wire_consumer_surface.rs` | 외부 사용 방식으로 version/variant/close error 처리 관측. 기존 파일 확장으로 충분하면 신규 파일 생략 |
| 문서 수정 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | 유효한 class와 OS label, version 범위, 오류 precedence, classifier 책임 명시 |
| 외부 소유 | 실제 product classifier·wire consumer repository | source/owner 미확인. 이 저장소 fixture와 배포 receipt 분리 |

## 구현 순서

1. B18/D03은 `PlanSource::new`, Serde, `TaskSpec::validate()`에 동일 ASCII identifier 표를 적용한다. 127/128/129 **UTF-8 byte** 길이, leading/trailing whitespace, NUL, 비ASCII의 `TaskIdentifierField`와 offset을 기록한다. `ClassificationRationale`을 임의 문자열 길이 검사 대상으로 바꾸지 않는다.
2. D13/D23은 writer의 roundtrip과 reader의 **미지원 입력**을 분리한다. `Snapshot.schema_version = 2`, decimal-string `u128`, raw `RuntimeConfig`/`TaskSpec`, legacy aliases는 기존 형태를 보존한다. 소비자 fixture는 지원 버전 집합을 명시하고 unknown `schema_version`, 미래 `AdmissionVerdict`/`TerminalReason` variant에서 typed decode/negotiation error를 반환한다. `Snapshot::conservation_violation()`은 독립 held 원장 검사가 아님을 문서화한다.
3. B24는 source/reason/class가 문법상 유효하지만 의미상 틀린 입력을 보낸다. 엔진이 저장·노출하는 provenance와 실제 classifier 판정을 분리한다. 외부 classifier가 없다면 integration acceptance를 보류한다.
4. D14는 유효한 `/`, `:`, 128-byte class가 원래 identity로 통과하면서 OS label만 bounded ASCII로 바뀌는 기존 control을 유지한다. NUL/비ASCII/129 byte는 build된 runtime의 admission **전** 오류와 worker counter 0으로 판정한다. 현재 library spec의 “NUL/exotic class runs” 문장은 허용된 class의 label 정규화와 구분해 고친다.
5. D17은 닫힌 Governor에 malformed `admit`/`admit_waitable`, foreign resolved capability의 `admit_resolved`, 유효 spec을 각각 넣는다. 현재 순서가 malformed/capability error를 우선할 수 있음을 고정하고 rustdoc의 “every admit*” 표현과 합치도록 결정한다. 오류 우선순위를 바꾼다면 S25-001에서 migration을 먼저 검토한다.

## DoD

- [ ] `S25-003-B18` custom/legacy `PlanSource`의 source API와 Serde spelling, UTF-8 byte 경계를 같은 표에서 검증한다.
- [ ] `S25-003-B24` 문법상 유효한 거짓 source/reason/class가 엔진에서 provenance로 보존되는 현재 경계와 classifier owner의 책임을 구분한다. 엔진 reject를 가짜 oracle로 쓰지 않는다.
- [ ] `S25-003-D03` identifier 길이 127/128/129 byte, whitespace/NUL/비ASCII와 정확한 `TaskIdentifierField`를 단언한다.
- [ ] `S25-003-D13` Snapshot version 2와 `u128` decimal string, raw config/spec, legacy alias를 roundtrip하고 Snapshot consumer의 version reject를 별도 판정한다.
- [ ] `S25-003-D14` 유효한 class의 thread label 정규화와 원래 provenance 보존, invalid class의 OS spawn 전 거절을 worker counter로 확인한다.
- [ ] `S25-003-D17` closed Governor에 malformed raw spec·foreign resolved capability·유효 spec을 각각 제출해 preflight와 `RuntimeUnavailable` 우선순위를 고정하고 rustdoc을 일치시킨다.
- [ ] `S25-003-D23` 미래 verdict/terminal variant가 구버전 consumer에서 decode 오류가 됨을 관측하고, 이를 처리하는 공개 consumer fixture를 둔다. Snapshot version으로 대체하지 않는다.

owner-local contract/engine/host → `test`, doctest/rustdoc, consumer MSRV, CI. wire breaking 변경은 S25-001의 migration 결정 없이 적용하지 않는다.

| Acceptance | 독립 oracle과 실패 조건 |
|---|---|
| `S25-003-B18` | source API/Serde 양쪽에서 custom/legacy spelling의 유효/무효가 일치하고, legacy alias 재직렬화가 값을 바꾸지 않음. provenance field가 enum rationale과 섞이지 않음. |
| `S25-003-B24` | 유효하지만 사실과 다른 metadata는 엔진에서 **그대로 관측**되고 accounting에는 의미 변경 0. classifier의 참값 검증은 지정된 외부 owner의 fixture/receipt. 엔진에서 거절시켜 K로 만들지 않음. |
| `S25-003-D03` | class, operation, root, parent, stage, reduce key 각각 exact field/byte offset; 128 byte 유효, 129 byte 실패. 공백/NUL/비ASCII는 길이와 별도 오류 축. |
| `S25-003-D13` | Snapshot v2만 consumer accept, 미래 버전 reject; held `u128`은 decimal string roundtrip; config/spec/legacy source도 원래 wire 유지. helper와 독립 ledger의 판정 범위를 혼동하지 않음. |
| `S25-003-D14` | 유효 identity와 OS thread label을 동시에 관측. invalid class는 `MalformedTask`, worker started 0, permit/ticket/pool 0. spawn 실패는 별도 typed 오류 control. |
| `S25-003-D17` | close 후 malformed/foreign/valid 각 API별 exact error와 side effect 0. 필요 시 `Governor::close_admission` rustdoc 수정, 소비자가 오류 precedence를 읽을 수 있음. |
| `S25-003-D23` | 미래 verdict와 terminal variant는 raw derived decoder에서 오류; 공개 소비자는 이를 panic/default-admit 없이 보고/협상. Snapshot version으로 enum 호환성을 가정하지 않음. |

## 계획된 검증

owner-local 후보: `cargo test --locked -p taskmesh-contract --test contract_roundtrip --test task_plan_validation --test snapshot_oracle --test error_surface`, `cargo test --locked -p taskmesh-engine --test provenance_preservation --test hardening_close_admission`, `cargo test --locked -p taskmesh --test hardening_executor_protocol`. 신규 target을 만들면 그 정확한 `--test wire_consumer_negatives`/`wire_consumer_surface`도 선택한다. 공개 문서/API 변경은 `just doctest`, `just rustdoc`, `just consumer-msrv`; S25-012의 기본 `just test`/CI 선택으로 인계한다. 이 문서 단계에서 명령은 실행하지 않는다.

## 인계·중단 조건

contract owner가 wire/error 변경을 단일 적용하고 engine·host owner는 재현 fixture 및 rustdoc patch를 넘긴다. `governor.rs`는 S25-004/008/009와 겹치므로 동시 수정하지 않는다. 실제 외부 classifier/consumer가 미확인이라면 이 저장소의 public-surface fixture는 완료 가능해도 배포 의미 검증은 OPEN이다. wire breaking, Snapshot 버전 증가, 오류 precedence 변경이 나오면 S25-001 migration 결정을 먼저 업데이트한다. 단순 roundtrip green으로 미래 consumer 호환을 닫지 않는다.
