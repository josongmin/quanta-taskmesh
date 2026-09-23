# SEP22-T02 cancellation handshakes

- 상태: LOCALLY_VERIFIED
- finding: TO-02
- priority: P1
- write lane: `cancellation-sequencing`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh/tests/cancellation_policy.rs`, `crates/taskmesh/tests/deadline_cancel.rs`
- 재감사 기준: `main@146233665942d75b73e2b724f781be7e105fd7c4` (2026-09-23)

## 목적

이미 도입된 first-poll/spawn handoff를 보존하고, 취소 결과와 fixture 종료까지 모두 이름 있는
시간 한도로 묶는다. 취소 기능이 회귀할 때 test binary 전체가 멈추지 않고 해당 경계에서 실패해야
한다.

## RCA

원래 finding의 20~50ms sequencing sleep은 현재 소스에서 이미 제거되었다. IO/local future의
첫 poll과 stalled `CpuExecutor::spawn` custody를 one-shot으로 관측한 뒤 token을 발화한다.

남은 결함은 terminal wait다. local work는 의도적으로 `pending()`에 머물며 취소만이 정상 종료
경로인데, `run_local_with(...).await`가 한도 없이 기다린다. stalled CPU executor는 closure와
완료 sender를 `held`에 보관하므로 취소 분기가 고장 나면 `handle.await`도 끝나지 않는다. 현재
`HANG` helper는 두 terminal wait에 적용되지 않는다. CPU test는 cancellation reply 후
`yield_now()`로 custody snapshot을 늦추고, held closure drop 뒤에는 bounded polling으로 drain을
확인한다. worker lease drop은 동기 release이므로 이 두 scheduling 단계도 필요성을 재검토해야 한다.

## 확정 근거

- IO policy 두 case는 first-poll one-shot과 bounded result join을 이미 사용한다:
  `crates/taskmesh/tests/cancellation_policy.rs:41-125`.
- local test는 start receipt만 bounded이고 `run_local_with` 결과와 firer join은 unbounded다:
  `crates/taskmesh/tests/deadline_cancel.rs:563-597`.
- CPU test는 executor acceptance receipt만 bounded이고 취소 결과 join은 unbounded다:
  `crates/taskmesh/tests/deadline_cancel.rs:605-683`.
- `run_local_with`는 caller future에서 `run_cancellable`을 사용한다:
  `crates/taskmesh/src/runtime.rs:761-792,1034-1083`.
- CPU path는 detached worker가 lease를 보유하고 cancellation reply와 실제 release를 분리한다:
  `crates/taskmesh/src/runtime.rs:711-759,887-942,1191-1225`.

## 목표 불변식

- token 발화 전 work future가 최소 한 번 poll되었거나 executor가 work를 custody했음이 증명된다.
- Cooperative는 mid-run cancel로 `GovernorError::Cancelled`를 반환한다.
- PreSubmitOnly는 같은 시점의 cancel을 무시하고 명시적 release 뒤 원래 값을 반환한다.
- local cancellation 결과와 firer join이 각각 bounded되고, `!Send` work는 현재 task에 남는다.
- stalled executor 취소 뒤 permit은 held work가 drop되기 전까지 1이고 drop 뒤 0이다.
- timeout failure path는 held closure를 해제하고 이름 있는 원인을 보고한다.

## 구현 플랜

1. 현재 owner input SHA-256을 다시 확인한다: `cancellation_policy.rs`는
   `0dadaef14fbe002734ab9d3622fc48fe1ce1d7f2fe4b006f9b8f24d6a79c7189`,
   `deadline_cancel.rs`는 `88cb0453de994ea6e1cea0fb0b3d565de53fb620ff38ad72d1905e5efee78351`.
   후자는 다른 writer의 `MutexGuard` 수정으로 dirty이므로 해당 diff를 보존한다.
2. local case에서 기존 `bounded(HANG, ...)` helper로 `run_local_with` 전체 결과를 감싼다.
   helper는 `Send` bound가 없어 `!Send` payload를 현재 task에서 그대로 poll한다. firer join도
   별도 이름으로 bounded한다. typed `Cancelled` assertion을 유지한다.
3. CPU case의 `JoinHandle`은 `&mut`로 `tokio::time::timeout(HANG, ...)`에 전달한다. timeout이면
   caller task를 abort하고 `held` closure를 drop한 뒤 해당 cancellation 경계 이름으로 실패한다.
   정상 경로는 `Cancelled`를 확인하고 held work가 남아 있는 동안 inflight 1을 즉시 assert한다.
4. `held.clear()`는 closure 안의 `ExecutionLease`를 동기 drop한다. 직후 inflight/queued 0과
   conservation을 확인한다. 이 즉시 assertion이 실패하면 기존 polling을 복원하지 않고 lease
   release owner를 조사한다.
5. IO policy case의 post-result `yield_now()`와 CPU case의 custody `yield_now()`를 제거하고
   terminal boundary에서 각각 정확한 accounting을 검사한다. PreSubmitOnly의 값 42와
   pre-submit reject control도 유지한다.
6. 기존 `bounded` helper 외에 범용 fixture 층이나 production cancellation 변경을 추가하지 않는다.

## 테스트와 intentional negative

- positive: Cooperative IO/local은 first poll 뒤 cancel로 종료되고, PreSubmitOnly는 release 뒤
  42를 반환한다.
- intentional negative: 임시 mutation에서 local `token.cancel()`을 제거하면 local result가
  `HANG` 안에 이름 있는 timeout으로 실패해야 한다.
- intentional negative: CPU token 발화를 제거하면 result join이 bounded failure를 내고 held
  closure가 cleanup된다. external CI timeout만으로 실패를 인정하지 않는다.
- custody negative: CPU cancellation reply 뒤 held work를 drop하기 전 inflight가 0이면 실패한다.
- release negative: held work drop 뒤 inflight/queued가 즉시 0이 아니면 실패한다.

## DoD

- SEP22-T02-A01 네 case의 first-poll/spawn receipt가 유지되고 scheduler sleep이 재도입되지 않는다.
- SEP22-T02-A02 Cooperative/PreSubmitOnly의 반대 결과와 local `!Send` 실행을 보존한다.
- SEP22-T02-A03 CPU custody 전후 accounting이 즉시 검증되고 timeout failure cleanup이 있다.
- SEP22-T02-A04 local result/firer와 CPU result join 모두 bounded이며 mutation이 내부 timeout을 증명한다.

## 금지되는 임시방편

- 더 짧은 sleep, 반복 `yield_now`, snapshot busy loop로 terminal wait를 숨기는 것.
- PreSubmitOnly work를 즉시 완료시켜 cancel과 경쟁시키는 것.
- cancellation policy, executor trait, production runtime을 테스트 편의로 바꾸는 것.
- CPU timeout에서 held closure를 살려 둔 채 detached caller만 버리는 것.
- 서로 다른 policy 결과를 하나의 느슨한 accepted set으로 합치는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test cancellation_policy -- --nocapture
cargo test --locked -p taskmesh --test deadline_cancel run_local_honors_cooperative_cancel -- --exact --nocapture
cargo test --locked -p taskmesh --test deadline_cancel run_cpu_escapes_stalled_executor_via_cancel -- --exact --nocapture
cargo test --locked -p taskmesh --test deadline_cancel -- --nocapture
```

각 exact filter의 executed count와 exit code를 기록한다. mutation probe는 별도 임시
worktree/target에서 수행하고 원상 복구한 source SHA를 확인한다. `just gate`, `just matrix`,
`just verify-macos-full`은 integrator 단계에서 현재 source에 대해 별도로 실행한다.

## Stop/reopen 조건

- first poll 이후 cancel인데 typed result가 정책 계약과 다르면 runtime cancellation owner로
  재개한다.
- held closure drop 직후 accounting이 nonzero면 timeout을 늘리지 않고 lease release owner로
  재개한다.
- `deadline_cancel.rs`의 기존 dirty owner diff가 바뀌면 입력 SHA와 write 범위를 다시 고정한다.

## 2026-09-23 구현 및 검증

- `cancellation_policy.rs`의 Cooperative 완료 후 불필요한 yield를 제거했다. Local 결과/firer와
  stalled CPU caller 결과를 각각 bounded wait로 묶었고 CPU timeout은 caller abort 및 held work
  drop을 수행한다. CPU custody와 held work drop 직후 accounting을 즉시 검사한다.
- owner source SHA-256: `cancellation_policy.rs` =
  `30b3a5f9898f39ee1744210aac216402c84959a946133453a178d961eea74494`,
  `deadline_cancel.rs` = `df0c0eeb816601611a8e66f634c7c4317c1fd47a450966d9f483ad1154668526`.
- owner-local rail: `cargo test --locked -p taskmesh --test cancellation_policy --test deadline_cancel --test e2e_scenarios`;
  front door `cargo test`, profile `test`, base HEAD `146233665942d75b73e2b724f781be7e105fd7c4`,
  dirty shared worktree, exit 0. Relevant binaries executed/passed: `cancellation_policy` 3/3,
  `deadline_cancel` 15/15, `e2e_scenarios` 9/9; failed/filtered/ignored 0.
- `rustfmt --check --edition 2021` on the three touched test files and `git diff --check` passed.
  Local/CPU `token.cancel()`을 각각 일시 제거한 exact test는 모두 1/1 실행·실패(exit 101)했다:
  local은 `local work did not return after cancellation` 5초 timeout, CPU는
  `caller did not return after cancellation within 5s` timeout이다. 각 mutation 직후 원복한
  `deadline_cancel.rs` SHA-256은 위 owner hash와 같다. 원복 뒤 15/15 재통과했다.
- 이 증거는 dirty shared worktree의 owner-local proof다. W3 exact-source qualification이 없어
  campaign closeout으로 승격하지 않는다.
