# SEP21-E02 — Atomic state commit and effect custody

- 상태: PLANNED
- 우선순위: P1
- 포함 finding: TM21-004
- 선행: 없음
- write lane: `engine-core`

## 목적

state transition과 host-owned waker/callback retirement 사이에 명시적 compensation boundary를
두어 wake 또는 destructor panic이 permit/ticket custody를 유실시키지 못하게 한다.

## RCA

- state는 mutex 안에서 commit되지만 last `Arc<dyn PermitWaker>` drop은 반환 경로에서
  무보호 실행된다.
- `wake()` panic과 destructor panic이 다른 경계로 처리된다.
- claim 결과 전달 전에 effect가 실패해도 ticket/permit을 되돌릴 protocol이 없다.

## 확정 근거

- claim이 ticket 제거/permit reservation을 commit한 뒤 `drop(waker)`를 실행:
  `crates/taskmesh-engine/src/engine/governor.rs:238-260`.
- `apply_effects`의 wake catch와 loop-variable destructor 경계:
  `crates/taskmesh-engine/src/engine/governor.rs:509-535`.
- host `TicketGuard` ownership 전이: `crates/taskmesh/src/runtime.rs:187-198,1348-1353`.

## 목표 구조와 불변식

- mutex 안에서는 pure state delta와 `TransitionEffects`만 생성한다.
- effect runner는 wake와 final drop을 각각 `catch_unwind`하고 batch 전체를 끝까지 drain한다.
- claim은 permit custody가 caller에게 전달되거나 engine이 compensating unwind 후 terminal
  outcome을 게시하는 둘 중 하나로 끝난다.
- first panic은 진단용으로 보존하되 accounting cleanup을 막지 않는다.

## 작업 플랜

1. `crates/taskmesh-engine/src/engine/governor.rs`
   - claim commit/result delivery/effect retirement 순서를 명시적 transaction으로 재구성한다.
   - 공통 `run_effects`가 wake와 destructor를 모두 격리한다.
2. `crates/taskmesh-engine/src/engine/state.rs`
   - claim delivery 실패 시 permit/ticket/capability/resource를 exactly-once unwind하는 state
     transition을 추가한다.
3. `crates/taskmesh-engine/src/lib.rs` 또는 engine-local outcome module
   - terminal reason을 engine-owned typed outcome으로 export한다. public contract 변경이 꼭
     필요하면 C01을 reopen하고 owner가 수정하며 E02가 contract 파일을 직접 수정하지 않는다.
4. host `TicketGuard`의 duplicate abandon/release 방지는 H02가 final acquisition arbiter와 함께
   통합한다. E02는 `crates/taskmesh/**`를 수정하지 않는다.

## 테스트 플랜

- `crates/taskmesh-engine/tests/hardening_effect_retirement.rs`
  - claim-time last-Arc Drop panic.
  - terminal waker Drop panic 뒤 같은 batch 후속 waiter wake.
  - wake panic + Drop panic + reentrant snapshot/claim/abandon/release.
- `crates/taskmesh-engine/tests/hardening_lifecycle.rs`
  - 모든 panic 경로 phase/resource conservation.
- host guard와 compensation의 duplicate release 방지는 H02 negative fixture다.

## DoD

- `SEP21-E02-A01`: panic 후 live orphan permit/ticket/queue entry가 0이다.
- `SEP21-E02-A02`: batch의 모든 non-panicking effect가 실행된다.
- `SEP21-E02-A03`: mutex 아래 callback/drop이 없고 reentrant test가 deadlock하지 않는다.
- `SEP21-E02-A04`: first panic의 typed/diagnostic 결과와 cleanup 결과가 deterministic하다.
- `SEP21-E02-A05`: normal claim hot path의 custody handoff가 두 번 실행되지 않는다.

## 금지되는 임시방편

- 모든 panic swallow.
- `mem::forget`, 무제한 retirement queue, background cleanup worker 추가.
- debug assertion을 cleanup 대신 사용.
- dead permit ID를 성공으로 반환.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh-engine --test hardening_effect_retirement --test hardening_lifecycle
```
