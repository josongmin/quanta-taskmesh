# H16-006 — Lock 밖 effect retirement와 bounded promotion

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-005](H16-005-memory-epochs-and-lease-clock.md)
- 원본 finding: [TM16-030](../../../../bugbash/sep-16-general/tickets/TM16-030-waker-destructor-under-governor-lock.md)
- 배타적 write lease: `engine`, `host`; [적용 순서](EXECUTION.md) 준수

## 목적

transition lock 안의 외부 wake/Drop을 제거하고 release/promotion/retirement가 bounded하며 진행성을 유지하게 한다.

## 변경 범위

- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh-engine/src/features/admission/mod.rs](../../../../../crates/taskmesh-engine/src/features/admission/mod.rs)
- 기존: [crates/taskmesh/src/adapters/permit_waker.rs](../../../../../crates/taskmesh/src/adapters/permit_waker.rs)
- 제안 경로: `crates/taskmesh-engine/src/engine/effects.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh-engine/tests/hardening_effect_retirement.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] state에서 제거한 외부 Arc/waker/retired payload를 TransitionEffects로 이동한다. 성공·reject·abandon·reap·construction failure 모든 final-drop 위치를 inventory화한다.
- [x] lock 밖에서 wake/retire를 실행한다. reentrant Drop/wake가 snapshot/release/admit을 호출하는 test를 추가한다.
- [x] 한 transition의 promotion 수 K와 scheduler visit budget을 명시한다. 아직 runnable work가 있으면 coalesced driver notification으로 continuation을 보장한다.
- [x] control event/release는 full data queue에 갇히지 않게 한다. completion·cancel은 per-live-request reserved cell/직접 짧은 transition으로 전달하고 drop하지 않는다.
  → 별도 control queue를 만들지 않고 제거: completion/cancel은 lock 아래 짧은 transition, 통지만 밖
- [x] slow/panicking custom callbacks는 contract fault로 진단한다. 별도 retirement worker를 쓰면 pending retired objects가 intake capacity에 계상되도록 하며 무한 큐를 만들지 않는다.
  → retirement worker 없음(N/A, EXCEPTIONS H16-006-A05); panicking waker는 격리되고 나머지 waiter는 모두 깨움(`every_waiter_in_one_pass_is_woken_even_if_an_earlier_one_panics`)
- [x] critical section에서는 OS spawn, formatting-heavy diagnostics, user allocator/callback을 통한 임의 작업을 피한다. hard real-time lock latency 보장은 하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `TransitionEffects`를 도입해 host `Arc`(waker)의 **wake와 destructor 모두**를 lock
  밖으로 옮겼다. abandon/reject/promote/unwind의 모든 final-drop 위치를 포함한다.
- promotion은 `PROMOTION_BUDGET` permit으로 bounded하고, 남은 runnable work가 있으면
  `more_runnable`로 보고해 caller가 lock 해제 후 재진입한다. 외부 event를 기다리지
  않으므로 진행성은 유지된다.
- control event용 별도 data queue는 **만들지 않았다**. 제거하는 쪽을 택했다:
  completion/cancel은 lock 아래 짧은 transition이고 통지만 밖에서 일어나므로 넘칠
  버퍼가 없다.

Regression: `crates/taskmesh-engine/tests/hardening_effect_retirement.rs` (4).
Mutation: waker를 lock 아래에서 drop하도록 되돌리면
`abandoning_a_queued_request_retires_its_waker_outside_the_lock`가 의도한 assertion으로
FAIL하고, 되돌림 해제 후 control은 PASS다.

## 검증 / 완료 조건

- [x] `H16-006-A01` 재진입 waker Drop probe가 deadlock 없이 종료
- [x] `H16-006-A02` K보다 많은 runnable이 추가 외부 release 없이 dispatch
- [x] `H16-006-A03` control 전달 경로에 넘칠 버퍼가 없음 (queue 제거로 해소)
- [x] `H16-006-A04` final owner Drop이 lock 안에서 발생하지 않음
- [ ] `H16-006-A05` retirement worker를 두지 않았으므로 retained count/bytes 상한은
      해당 없음. slow callback은 호출한 스레드를 점유할 뿐 engine state를 잠그지 않는다.
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [x] `H16-006-A06` callback panic 정책: `wake()`/destructor panic은 `catch_unwind`로 격리되어
      같은 pass의 나머지 waiter가 모두 통지된 뒤 첫 panic이 전파된다
      (`every_waiter_in_one_pass_is_woken_even_if_an_earlier_one_panics`; mutation
      `panicking-waker-aborts-the-wake-loop`). 감사(A1-P1-1) 후 promotion budget은 `select` *전에*
      검사되어 64번째 선택이 dispatch 없이 fairness 비용을 남기지 않는다
      (`drr_order_is_exact_across_the_promotion_budget_boundary`).

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test admission_queue --test concurrency_stress
cargo test -p taskmesh --test runtime_cancel_timeout
```

## 호환성 / 실패 모드

- effects queue를 추가하는 것만으로는 boundedness가 생기지 않는다.
- 완료 notification은 관측 로그와 달리 best-effort drop 대상이 아니다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

