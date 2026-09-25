# BG25-006 — deadline response와 worker custody

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P0
- 선행: BG25-001 D6, BG25-005
- 소유: host/runtime owner; engine lease 변경은 engine owner

## 목적

caller response, root completion, owned-runtime teardown, blocking child termination, lease release, drain을 독립 사건으로 만든다. `RunFor`와 `CompleteBy`를 섞지 않고 정상 `Ok`와 task `Err` 모두 같은 response arbiter를 사용한다.

## 근거

- 감사 기준의 requested-stack 정상 success/task error는 runtime drop 뒤 전송돼 blocking child가 응답을 지연했다.
- 현재 `CompleteBy`는 정상 `Ok`와 task `Err`에도 caller 응답 deadline을 적용하고 worker custody는 실제 teardown까지 유지한다.
- `host_open_loop::caller_terminal_response_and_worker_custody_are_separate_ledgers`가 개별 deadline/cancel/direct-lease fixture와 별도로 응답·worker·lease ledger를 결합한다.

## 변경 파일

- `crates/taskmesh/src/runtime.rs`, 조건부 `execution_plan.rs`
- tests: `hardening_deadline_custody.rs`, `deadline_cancel.rs`, `hardening_drain.rs`, `runtime_cpu_executor.rs`
- `host_open_loop.rs` response/custody ledger
- public deadline/custody docs

## 작업 계획

1. root-start/result-ready/response/child-active/teardown/refund를 barrier와 독립 event ledger로 기록한다.
2. `CompleteBy` terminal response owner를 cleanup owner와 분리하고 late success/error를 한 번만 폐기한다.
3. `RunFor`는 root 실행 예산과 release-fenced late result를 유지한다.
4. accepted sync closure와 direct external lease preemption을 second admission/drain과 교차한다.

## DoD

- `BG25-006-D05`: ZERO/equality/checked-add overflow가 async와 started sync 계약에 맞게 분리된다.
- `BG25-006-D15`: stack 0/1/MAX/MAX+1/usize overflow를 각 dispatch path에서 typed preflight 또는 typed spawn failure로 판정한다.
- `BG25-006-H11`: cancel→absolute→relative precedence, equality expiry, late admission unwind, closure start 0을 검증한다.
- `BG25-006-H12`: started sync worker 뒤 deadline/cancel/drop 중 capacity 재판매와 drain 성공이 없다.
- `BG25-006-H19`: accepted-held CPU closure가 execute/drop 전까지 charged이고 `RunFor`가 시작되지 않는다.
- `BG25-006-H20`: deadline/panic response는 live blocking child보다 먼저지만 lease는 child 종료까지 유지된다.
- `BG25-006-H31`: external lease preemption 시 host work 0, host drop refund 0, token release 전 drain 불가다.
- `BG25-006-H34`: 정상 `Ok`와 task `Err` 각각에서 `RunFor` late result와 `CompleteBy` terminal response 계약을 검증한다.

## 검증

- Fixed sleep 대신 barrier/clock 사용; unavoidable timing에는 여유와 hard upper bound를 분리한다.
- Focused host tests 후 BG25-012 CI profile.

## 현재 완료 범위

- H34: requested-stack `CompleteBy`에서 teardown이 deadline을 넘으면 준비된 `Ok`/task `Err`를 폐기하고 `DeadlineExceeded`를 반환한다. worker는 실제 teardown까지 lease를 보유한다.
- Tokio timer/local service가 필요한 경로는 permit/ticket 전에 typed preflight를 수행한다. pre-cancel/expired deadline처럼 이미 결정된 계약 verdict는 host prerequisite보다 먼저 반환한다.
- `hardening_deadline_custody`와 `hardening_executor_protocol`의 default/Rayon focused control 및 `just dev`가 통과했다.
- D05, D15, H11, H12, H19, H20, H31의 개별 fixture에 `host_open_loop::caller_terminal_response_and_worker_custody_are_separate_ledgers`의 결합 ledger를 추가했다.
- 관련 default tests와 unit tests는 최종 committed HEAD의 BG25-012 receipt에서 함께 판정한다.
- D05는 checked-add overflow, zero-budget try-once, deadline equality를 별도 case로 연결한다. D15는 0/1/MAX/MAX+1/target-usize validator 경계와 sync/async preflight case를 연결한다.
- H11은 cancel/deadline/acquire의 동일 tick 우선순위 unit cases를 연결한다. H12/H19는 caller 응답, accepted/started worker custody, RunFor, 다른 요청과 drain의 별도 fixture를 supporting cases로 연결한다.

## 인계 및 중단 조건

- 실제 worker/child 종료 전에 lease를 풀거나 terminal 뒤 두 번째 응답이 관측되면 중단한다.
- `CompleteBy` 의미가 승인되지 않으면 구현하지 않고 BG25-001로 되돌린다.
