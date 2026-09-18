# H16-012 — Deadline·completion fence·cleanup custody

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: R — Runtime/adapter owner (실제 assignee 미지정)
- 선행 완료: [H16-011](H16-011-executor-protocol-and-setup.md)
- 원본 finding: [TM16-002](../../../bugbash/sep-16-general/tickets/TM16-002-blocking-runfor-deadline-is-inert.md), [TM16-015](../../../bugbash/sep-16-general/tickets/TM16-015-requested-stack-completion-before-release.md), [TM16-022](../../../bugbash/sep-16-general/tickets/TM16-022-cpu-relative-deadline-starts-after-spawn.md), [TM16-024](../../../bugbash/sep-16-general/tickets/TM16-024-requested-stack-shutdown-delays-deadline.md), [TM16-032](../../../bugbash/sep-16-general/tickets/TM16-032-acquire-timeout-ignores-admission-lock-wait.md)
- 배타적 write lease: `host`, `engine`, `contract`; [적용 순서](EXECUTION.md) 준수

## 목적

acquire/run/response/termination clock과 cancellation winner를 고정하고 cleanup 중 용량을 보존한다.

## 변경 범위

- 기존: [crates/taskmesh/src/runtime.rs](../../../../crates/taskmesh/src/runtime.rs)
- 기존: [crates/taskmesh/src/executor/cancel.rs](../../../../crates/taskmesh/src/executor/cancel.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh/tests/deadline_cancel.rs](../../../../crates/taskmesh/tests/deadline_cancel.rs)
- 기존: [crates/taskmesh/tests/cancellation_policy.rs](../../../../crates/taskmesh/tests/cancellation_policy.rs)
- 제안 경로: `crates/taskmesh/tests/hardening_deadline_custody.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] Acquire deadline은 queue 입장·promotion claim·worker start authorization에서 재검사한다. lock wait 후 immediate Admitted와 timeout last-chance claim도 우회하지 못한다.
- [x] ZERO acquire는 wait하지 않는 try operation으로 별도 처리한다. free capacity 즉시 실행 허용 여부를 D09에 고정하고 now+0 generic deadline 때문에 전부 거절하지 않는다.
- [x] RunFor 시작은 worker-entry timestamp로 보존하고 inline executor completion도 동일 authority로 판정한다. completion-at-deadline tie는 deadline wins를 권장한다.
- [x] normal/requested-stack blocking RunFor는 unsupported 사전 reject 또는 명시적 caller-wait deadline 중 D09 선택을 구현한다. CompleteBy unsupported 경로는 기존 사전 reject를 보존한다.
- [x] 정상 success/task-error 결과의 completion fence는 worker-owned runtime/context·지원 child cleanup·lease release 범위를 D10으로 고정한다.
- [x] deadline/cancel 응답은 cleanup 이전에 전달할 수 있으나 worker/cleanup owner가 semantic+dispatch credit을 유지한다. bounded cleanup count는 active execution 한도에 포함한다.
- [x] shutdown/drain timeout은 StillRunning/NotDrained로 보고한다. shutdown_timeout/background 반환을 termination receipt로 사용하지 않는다.
  → `TokioRuntime::drain(timeout)` → `Err(NotDrained { classes, elapsed })` (D17); drain은 reply가 아니라 engine custody gauge를 보고, timeout 반환은 termination receipt가 아니다(runtime은 draining으로 남는다)
- [x] completion timestamp와 cleanup/response latency를 별도 기록한다. timely completion의 late delivery 처리와 CompleteBy 전체범위의 차이를 문서화한다.
  → worker가 start/completion timestamp를 결과와 함께 보내 판정; 별도 latency metric은 두지 않았고 response≠custody는 D10에 문서화

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- acquisition budget이 **모든** 대기를 포함한다. 즉시 승인된 permit도 budget 만료 후면
  unwind하고 `PermitAcquireTimedOut`을 반환한다. `Duration::ZERO`는 기존 try 의미를
  유지한다(D09).
- `RunFor`는 worker 자신의 시작 시각을 기준으로 한다. worker가 start/completion
  timestamp를 함께 보내므로 inline executor도 같은 authority로 판정된다. tie는 deadline이
  이긴다.
- blocking `RunFor`는 caller-wait deadline으로 구현했다(D10). 시작된 동기 작업의 종료를
  보장한다고 표현하지 않으며 worker는 계속 charged 상태다.
- 정상 완료 시 custody가 결과와 함께 이동한다 — caller가 값을 볼 수 있으면 용량은 이미
  반환되어 있다. deadline/cancel 같은 terminal 경로는 응답을 **먼저** 보내고 teardown은
  lease를 쥔 채 진행한다.

Regression: `crates/taskmesh/tests/hardening_deadline_custody.rs` (10),
`crates/taskmesh/tests/deadline_cancel.rs` (12).

## 검증 / 완료 조건

- [x] `H16-012-A01` 만료된 acquisition budget에서 job starts=0
- [x] `H16-012-A02` inline 60ms job + 5ms RunFor가 늦은 Ok로 반환되지 않음
- [x] `H16-012-A03` promotion-after-expiry, cancel-vs-complete, ZERO/free-capacity matrix
- [x] `H16-012-A04` yielding root + blocking child에서 deadline 응답 먼저, cleanup 중
      credit 유지, barrier 해제 후 refund 1회
- [x] `H16-012-A05` requested-stack 성공 후 즉시 snapshot 및 연속 ZERO acquire fence
- [x] `H16-012-A06` caller drop/worker panic/task error에서 무계상 실행 없음
- [x] `H16-012-A07` `TokioRuntime::drain(timeout)`이 timeout에 `NotDrained`를 클래스별
      `inflight`/`queued`로 보고한다 (`a_held_blocking_worker_makes_drain_report_not_drained_and_stay_charged`,
      `not_drained_lists_each_class_with_its_own_inflight_and_queued_counts`); 보고 뒤에도
      worker는 charged 상태로 남고 runtime은 draining이다. teardown은 여전히 handle drop뿐 (ADR 0003 D17)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh --test deadline_cancel --test cancellation_policy --test runtime_cancel_leak --test runtime_cpu_executor
```

## 호환성 / 실패 모드

- OS preemption 없는 hard response-latency upper bound는 제공하지 않는다.
- 자식 작업 전체가 추적되지 않는 ambient Tokio 경로에서 universal structured concurrency를 약속하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-18.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
