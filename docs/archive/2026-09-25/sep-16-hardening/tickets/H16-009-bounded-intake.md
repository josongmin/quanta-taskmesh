# H16-009 — 대기 이전 bounded intake와 pending accounting

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: R — Runtime/adapter owner (실제 assignee 미지정)
- 선행 완료: [H16-006](H16-006-effects-and-bounded-promotion.md), [H16-008](H16-008-resolved-execution-plan.md)
- 원본 finding: [TM16-001](../../../../bugbash/sep-16-general/tickets/TM16-001-substrate-waiters-bypass-bounded-admission.md)
- 배타적 write lease: `engine`, `host`, `contract`; [적용 순서](EXECUTION.md) 준수

## 목적

first-poll admission以前의 caller memory와 accepted work를 구분하고, queue에 들어가기 전 즉시 bounded reservation을 확보한다.

## 변경 범위

- 기존: [crates/taskmesh/src/runtime.rs](../../../../../crates/taskmesh/src/runtime.rs)
- 기존: [crates/taskmesh/src/executor/cancel.rs](../../../../../crates/taskmesh/src/executor/cancel.rs)
- 기존: [crates/taskmesh-engine/src/features/admission/mod.rs](../../../../../crates/taskmesh-engine/src/features/admission/mod.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../../crates/taskmesh-engine/src/engine/state.rs)
- 제안 경로: `crates/taskmesh/src/intake.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh/tests/hardening_intake_bounds.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] D01에서 ownership boundary를 first poll로 고정한다. 미poll Future/closure는 caller-owned임을 문서화하고 api-created wrapper가 무한 runtime registry에 등록되지 않게 한다.
- [x] global/class/capability pending credits를 try 방식으로 얻는다. 등록 가능한 수를 초과한 요청은 send().await/semaphore wait queue에 쌓지 않고 typed overload로 즉시 반환한다.
- [x] metadata byte length와 stage count를 검증한다. closure-captured heap은 자동 측정할 수 없으므로 declared payload budget과 외부 memory 책임을 구분한다.
  → stage count/구조는 `MalformedTask`로 검증; metadata byte budget·declared payload bound는 도입하지 않음(D01, EXCEPTIONS)
- [x] PendingGuard가 quota를 소유하고 accept/cancel/deadline/drop에서 request record와 함께 이전/반환한다. Pending/DispatchReserved/Accepted/Running/CleanupPending 및 Terminal metadata의 상한과 별도 ResultHeld retention bound를 선언한다. adapter accept 뒤에는 시작 전이라도 종료 확인 없이 credit을 반환하지 않는다.
  → ResultHeld retention은 없다(결과는 caller future로 이동); terminal record retention만 `MAX_TERMINAL_TICKETS`로 bounded
- [x] 기존 max_queue_depth/OverflowPolicy::Reject와 pending cap의 관계를 명시한다. Reject는 semantic 또는 worker capacity가 즉시 불가하면 기다리지 않는다.
- [x] borrowed run_io와 !Send run_local work는 caller-side future에 보존한다. central queue에는 실행 metadata/notification만 이동한다.
- [x] 선언된 parent-child wait와 unknown nesting을 D12 계약에 연결한다. bounded queue 자체를 deadlock avoidance라고 주장하지 않는다.
  → D12에 문서화; 선언된 nested cycle typed reject는 미구현(EXCEPTIONS H16-010-A06)

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- host의 `SubstrateGates` semaphore를 **제거**했다. capability 점유는 이제 class
  inflight·resource budget과 같은 admission transition에서 결정된다.
- 따라서 admission 앞에 무제한 대기 공간이 없다: `Reject`는 즉시 거절하고,
  `queued`는 실제로 기다리는 것을 센다.
- capability 포화로 큐에 들어간 요청은 `blocked_on`을 기록하므로 acquire timeout이
  `SubstratePoolTimedOut`과 `PermitAcquireTimedOut`을 구별해 보고한다.
- 대기 중인 요청은 worker slot을 점유하지 않는다 — 다른 runnable 클래스가 굶지 않는다.
- borrowed `run_io`와 `!Send` `run_local` payload는 caller future에 보존했다.

Regression: `crates/taskmesh/tests/hardening_intake_bounds.rs` (4),
`crates/taskmesh/tests/host_inferno.rs`의 2개 substrate 테스트. intake 검증은 정확한 verdict
(`CpuSaturated`)를 단언한다.

## 검증 / 완료 조건

- [x] `H16-009-A01` burst 후 accepted pending이 모든 상한 이내
- [x] `H16-009-A02` Reject/unknown이 held-worker 뒤에 parked되지 않음
- [x] `H16-009-A03` 반복 제출/timeout에서 보유량 확인
- [x] `H16-009-A04` 대기 중 작업이 idle execution credit을 점유하지 않음
- [x] `H16-009-A05` drop/cancel/deadline에서 quota가 한 번만 반환
- [x] `H16-009-A06` borrowed `run_io` / `Rc` `run_local` compile·실행 보존
- [ ] metadata byte budget과 declared payload bound는 도입하지 않았다. 제한하는 대상은
      runtime-owned outstanding work이며, caller가 만든 임의 heap을 hard-cap한다고
      표현하지 않는다 (D01).
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh --test substrate_enforcement --test runtime_cancel_timeout --test runtime_local --test runtime_io
```

## 호환성 / 실패 모드

- 제한하는 것은 runtime-owned outstanding work다. caller가 만든 arbitrary heap 전체를 hard-cap한다고 표현하지 않는다.
- non-Send payload를 중앙 Send+'static actor mailbox로 옮기는 재작성 금지.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
