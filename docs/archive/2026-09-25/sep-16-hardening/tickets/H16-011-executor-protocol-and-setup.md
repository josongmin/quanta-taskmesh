# H16-011 — Executor ownership protocol·worker setup 오류

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: R — Runtime/adapter owner (실제 assignee 미지정)
- 선행 완료: [H16-009](H16-009-bounded-intake.md)
- 원본 finding: [TM16-031](../../../../bugbash/sep-16-general/tickets/TM16-031-class-nul-panics-thread-construction.md)
- 배타적 write lease: `contract`, `host`, `rayon`; [적용 순서](EXECUTION.md) 준수

## 목적

executor 인계 전/후 실패를 구분하고 accepted work와 lease의 단독 소유자를 보존한다.

## 변경 범위

- 기존: [crates/taskmesh-contract/src/ports.rs](../../../../../crates/taskmesh-contract/src/ports.rs)
- 기존: [crates/taskmesh/src/executor/mod.rs](../../../../../crates/taskmesh/src/executor/mod.rs)
- 기존: [crates/taskmesh/src/executor/tokio_exec.rs](../../../../../crates/taskmesh/src/executor/tokio_exec.rs)
- 기존: [crates/taskmesh/src/runtime.rs](../../../../../crates/taskmesh/src/runtime.rs)
- 기존: [crates/taskmesh-rayon/src/lib.rs](../../../../../crates/taskmesh-rayon/src/lib.rs)
- 제안 경로: `crates/taskmesh/src/executor/dispatch_protocol.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh/tests/hardening_executor_protocol.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] try_reserve→submit→Accepted→Started→Terminated protocol을 정의한다. job/token 반환 가능한 NotAccepted와 accepted-but-outcome-unknown을 구분한다.
- [x] enqueue 이후 panic을 not-started로 취급해 재제출하지 않는다. recovery가 불가능한 adapter는 fault/quarantine 상태에서 admission을 중단하고 original ownership을 유지한다.
  → quarantine 상태는 두지 않았다: adapter fault는 typed(`WorkerUnavailable`/`JobAbandoned`)로 그 제출에 보고되고 재제출하지 않는다
- [x] 동일 Arc executor를 runtime 여러 개가 쓸 때 공유 capacity authority를 사용한다. default Tokio는 ambient pool의 외부 작업까지 통제하지 못한다는 managed-credit 한계를 노출한다.
  → `two_runtimes_sharing_an_executor_each_govern_their_own_submissions`; ambient Tokio pool의 외부 작업 미통제는 README/ADR에 명시
- [x] legacy CpuExecutor adapter는 conservative capability profile을 선언한다. unknown capacity·blocking submit에서 strict guarantee를 허위 제공하지 않는다.
- [x] worker entry를 공통 wrapper로 통합하여 start timestamp/authorization, task error/panic, result delivery, termination receipt를 기록한다. 실제 adapter별 spawn/affinity 차이는 유지한다.
- [x] OS thread label은 class identity와 분리해 sanitize/size bound한다. stack conversion/name/build/spawn/setup 오류를 typed boundary로 반환한다.
- [x] public RunError::Task(E)는 원본을 유지한다. PolicyViolation 문자열 남발 대신 error kind+phase+safe context를 추가하며 normal receiver drop은 명시적 expected delivery outcome으로 다룬다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- 모든 detached dispatch(blocking pool / CPU executor port / 전용 stack thread)가
  `run_detached_job` 하나를 공유한다: worker 시작 시각, phase 전이, panic 경계,
  lease 인계가 세 경로에서 동일하다. 실행 위치의 차이만 남긴다.
- `CpuExecutor::capabilities()`를 default 구현과 함께 추가했다. 기본값은 conservative
  (`legacy`)이며 host는 adapter가 선언한 것 이상을 가정하지 않는다. **감사(A2-P0-2) 후
  load-bearing**: `Builder::build`가 `declared_workers < cpu gate`를
  `TopologyError::ExecutorDeclaresFewerWorkers`로 거절하고, Rayon/BlockingPool adapter가 실제
  프로필을 선언하며, `TokioRuntime::executor_capabilities()`가 노출한다
  (`a_cpu_gate_wider_than_the_executor_declares_is_refused_at_build`; mutation `executor-declaration-ignored`).
- adapter가 job을 받은 뒤 panic하면 **재제출하지 않는다**. closure가 이미 adapter
  소유이므로 재제출은 caller 작업의 두 번째 실행이 된다.
- OS thread label을 sanitize·bounded하게 생성한다. class identity 자체는 바꾸지 않는다.
- task error / worker panic / spawn 실패 / receiver 소멸을 각각 구별해 보고한다 — 감사 후
  typed: `WorkerPanicked { context }` / `WorkerUnavailable { context, detail }`(`stack_size_bytes(u64::MAX)`로
  실제 spawn 실패를 test) / `JobAbandoned { context }`. receiver 소멸은 **정상적인** 전달 결과이며
  그때는 worker가 release 소유자다. Accepted-but-not-started에서 caller가 떠나도 permit·gauge가
  유지되고 한 번만 refund됨을 holding adapter로 test한다.

Regression: `crates/taskmesh/tests/hardening_executor_protocol.rs` (8).
Mutation: thread label sanitization을 제거하면 원본 티켓과 동일한
`thread name may not contain interior null bytes` panic으로 FAIL한다.

## 검증 / 완료 조건

- [x] `H16-011-A01` inline/delayed/reject-before-accept/enqueue-then-panic에서 실행 ≤1회
- [x] `H16-011-A02` caller가 대기를 멈춰도 queued job 소유권이 조기 해제되지 않음
- [x] `H16-011-A03` 중복/역순 phase 선언이 accounting을 두 번 바꾸지 않음
- [x] `H16-011-A04` NUL/긴/Unicode/빈 class에서 caller panic 없음
- [x] `H16-011-A05` 공유 executor의 선언된 한계 시험 (선언은 builder가 검증·runtime이 노출)
- [x] `H16-011-A06` task error/panic/spawn 실패/receiver 소멸 각각 별도 typed outcome
      (`WorkerPanicked`/`WorkerUnavailable`/`JobAbandoned`; `each_failure_mode_is_reported_as_itself`)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh --test runtime_cpu_executor --test runtime_blocking --test runtime_cpu
cargo test -p taskmesh-rayon --test rayon_smoke
```

## 호환성 / 실패 모드

- 일반 Rust thread를 안전하게 강제 kill하는 기능은 추가하지 않는다.
- legacy trait의 blocking spawn을 async timer로 선점 가능하다고 주장하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

