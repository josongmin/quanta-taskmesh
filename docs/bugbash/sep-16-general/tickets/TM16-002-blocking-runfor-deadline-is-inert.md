# TM16-002 — Blocking 경로가 CooperativeWithDeadline의 RunFor를 집행하지 않는다

- Severity: P2
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh/src/runtime.rs:469-520,134-175; docs/taskmesh-library-spec.md:82-93; crates/taskmesh/src/executor/cancel.rs:36-38

## Trigger / 관찰

`CooperativeWithDeadline` class의 `run_blocking_with`에 `with_deadline(5ms)`와 60ms 작업을 제출한다. requested-stack blocking도 같은 옵션 경로다.

일반 blocking 재현은 50ms 이상 경과한 후 `Ok(7)`을 반환한다. blocking은 `cancel_controls`/`run_cancellable`을 사용하지 않아 RunFor가 inert하다. CompleteBy는 해당 class에서 사전 거부하므로 이 지적의 대상이 아니다.

## 원인 / 영향 / 범위

동기 작업을 강제 중단할 수 없다는 제약과 deadline 옵션의 허용 여부가 명확하지 않다. CPU는 caller wait를 deadline으로 중단하면서 worker lease를 유지하지만 blocking은 caller wait도 제한하지 않는다. 문서의 mid-run 지원 목록은 run_io/run_local/run_cpu이며 blocking 지원은 명시적이지 않으므로, 이 티켓은 accepted-but-inert 옵션/문서 계약 문제다. blocking worker 강제 종료가 이미 보장됐다는 결론은 아니다.

## 보완 계획

blocking의 RunFor 계약을 선택해 고정한다: unsupported면 submit 전에 PolicyViolation으로 거부하고 문서화하거나, caller wait timeout을 제공하되 worker가 종료할 때까지 ExecutionLease를 보존한다. 동기 작업 종료를 보장한다고 표현하지 않는다.

## Acceptance / 회귀 검증

일반/requested-stack blocking 각각 RunFor 만료, 이미 종료한 작업, timeout 후 실제 worker lease 유지, worker 종료 후 회수, CompleteBy 사전 reject를 검증한다.
