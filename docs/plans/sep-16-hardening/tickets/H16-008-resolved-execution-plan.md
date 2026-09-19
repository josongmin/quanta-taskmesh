# H16-008 — Resolved execution plan과 실제 capability 일치

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: R — Runtime/adapter owner (실제 assignee 미지정)
- 선행 완료: [H16-002](H16-002-validated-policy-topology.md)
- 원본 finding: [TM16-023](../../../bugbash/sep-16-general/tickets/TM16-023-stack-dispatch-bypasses-capability.md)
- 배타적 write lease: `contract`, `host`; [적용 순서](EXECUTION.md) 준수

## 목적

hint·stack request·fallback·cancel controls의 해석을 단일 ResolvedExecutionPlan으로 고정한다.

## 변경 범위

- 기존: [crates/taskmesh/src/runtime.rs](../../../../crates/taskmesh/src/runtime.rs)
- 기존: [crates/taskmesh/src/builder.rs](../../../../crates/taskmesh/src/builder.rs)
- 기존: [crates/taskmesh-contract/src/task.rs](../../../../crates/taskmesh-contract/src/task.rs)
- 기존: [crates/taskmesh-engine/src/features/admission/mod.rs](../../../../crates/taskmesh-engine/src/features/admission/mod.rs)
- 제안 경로: `crates/taskmesh/src/execution_plan.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh/tests/hardening_dispatch_resolution.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] declared_class/effective_class, physical capability set, adapter identity, cost, cancellation/deadline support, config_epoch를 담는 immutable execution plan을 도입한다.
- [x] hint와 stack bytes를 함께 resolve한다. blocking/background+stack은 D04에 따라 실제 large-stack capability로 resolve하거나 사전 reject한다.
- [x] fallback 선택은 memory 상태를 보는 admission transition에서 결정하고 최종 resolved plan을 host로 반환한다. queue에 들어갈 때 effective authority를 freeze하고 나중에 original class로 다시 해석하지 않는다.
- [x] D03 권장안은 resource-only reclassification을 explicit하게 이름 붙이고 controls는 선언 class에 유지하는 것이다. full-policy 전환은 별도 호환성 승인 없이는 도입하지 않는다.
- [x] run_io/run_local/run_cpu/blocking/requested-stack의 duplicated cancel_controls/absolute_deadline lookup을 공통 resolver로 옮긴다.
- [x] config는 runtime lifetime동안 immutable epoch를 사용한다. hot reload나 request mid-flight reclassification은 이번 작업에 추가하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `crates/taskmesh/src/execution_plan.rs`에 `ResolvedExecutionPlan`을 추가했다. 진입
  경로마다 흩어져 있던 hint 검사·capability 선택·cancel/deadline lookup을 한 곳에서
  한 번 resolve한다.
- stack 요청이 있는 blocking-family 제출은 선언 hint와 무관하게 `large_stack`
  capability를 예약한다(D04). 실제로 실행되는 pool과 예약되는 pool이 일치한다.
- resolve된 capability는 intake에서 pending record에 **동결**되므로 promotion이 더 싼
  pool로 재해석할 수 없다.
- fallback은 자원 재분류만 수행하고 controls는 선언 클래스에 남는다(D03). 선언 클래스는
  caller의 `TaskSpec`에, 유효 클래스는 `permit_ledger(permit).class`에 있다 — 세 번째
  사본은 두지 않았다.

Regression: `crates/taskmesh/tests/hardening_dispatch_resolution.rs` (4).

## 검증 / 완료 조건

- [x] `H16-008-A01` blocking/background/large-stack × stack Some/None × cap1 matrix
- [x] `H16-008-A02` invalid hint는 physical wait 및 사용자 작업 전에 reject
- [x] `H16-008-A03` fallback의 자원 class와 controls 출처가 구별됨
- [x] `H16-008-A04` 같은 closure를 실행하며 작업량이 줄었다고 보고하지 않음
- [x] `H16-008-A05` non-Send local future는 caller thread에 남고 metadata만 resolve

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh --test substrate_enforcement --test runtime_cpu_executor --test cancellation_policy
cargo test -p taskmesh-engine --test memory_overcommit
```

## 호환성 / 실패 모드

- fallback은 정적 preflight에서 항상 확정할 수 없다. validation과 동적 effective-class 선택을 구분한다.
- public SubstrateHint enum을 엔진별 pool proliferation으로 확장하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

