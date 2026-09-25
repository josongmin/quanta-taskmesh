# SEP22-T03 backpressure handshake

- 상태: LOCALLY_VERIFIED (전체 qualification은 W3)
- finding: TO-03
- priority: P1
- write lane: `e2e-scenarios`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh/tests/e2e_scenarios.rs`
- 재감사 기준: `main@146233665942d75b73e2b724f781be7e105fd7c4` (2026-09-23)

## 목적

이미 추가된 holder-start/first-saturation handshake를 유지하면서 재시도 허가 경계를 실제
holder 완료 뒤로 옮긴다. 첫 요청은 정확히 한 번 reject되고 두 번째 요청은 정확히 한 번 성공해야
한다.

## RCA

원래 50/80/5ms sleep과 최대 1000회 retry는 현재 소스에서 제거되었다. holder 시작과 최초
`CpuSaturated`는 one-shot으로 관측한다. 그러나 test는 holder에게 release 신호를 보낸 직후
client의 retry gate를 열고, holder future/lease가 실제 끝났는지는 두 task를 `join!`할 때까지
확인하지 않는다. 따라서 두 번째 시도가 holder의 permit release보다 먼저 실행될 수 있다. 이를
흡수하려고 client가 무한 루프로 다시 제출하고, 후속 saturation마다 `yield_now()`를 실행한다.
30초 outer timeout은 이 회전을 끝내 줄 뿐 원인을 제거하지 않는다. `attempts > 0`도 정확히 한 번의
재시도나 결과 값을 증명하지 않는다.

또한 기존 주석과 negative oracle은 `max_queue_depth(0)`이 즉시 거절의 원인이라고 적는다.
실제로 `ClassPolicy::default()`의 overflow policy가 `Reject`이고, inflight blocker는 그 정책에
따라 `CpuSaturated`로 투영된다. queue depth만 1로 바꾸어도 기본 `Reject`는 그대로다.

## 확정 근거

- holder는 blocking closure 진입을 one-shot으로 알린다:
  `crates/taskmesh/tests/e2e_scenarios.rs:292-326`.
- client는 최초 saturation 뒤 retry gate를 기다리지만 그 후에는 무한 loop/yield를 사용한다:
  `crates/taskmesh/tests/e2e_scenarios.rs:328-362`.
- release와 retry 허가는 holder join보다 먼저 발생하며 최종 oracle은 `attempts > 0`이다:
  `crates/taskmesh/tests/e2e_scenarios.rs:365-384`.
- 기본 overflow policy는 `Reject`다: `crates/taskmesh-contract/src/policy.rs:178-203`.
- inflight block의 즉시 rejection은 overflow policy가 결정하고 class blocker는
  `CpuSaturated`로 매핑된다: `crates/taskmesh-engine/src/features/admission/mod.rs:50-65,265-277`.
- blocking worker의 정상 결과는 lease drop 이후 caller에 보인다:
  `crates/taskmesh/src/runtime.rs:887-942`.

## 목표 불변식

- 첫 client 제출 전 holder가 실제 blocking closure에 진입해 유일한 slot을 점유한다.
- class overflow는 명시적 `Reject`; 최초 client 제출은 정확히 `CpuSaturated`로 실패하고
  rejected work closure는 실행되지 않는다.
- 최초 rejection을 확인한 뒤 holder를 release하고 holder future 완료를 기다린다.
- holder completion 뒤 class slot이 비었고, 두 번째 client 제출은 한 번에 정해진 값을 반환한다.
- start, 첫 결과, holder join, 두 번째 결과는 각각 이름 있는 bounded wait이며 final drain은 exact다.

## 구현 플랜

1. 현재 owner input SHA-256 `4ebed4d41c788c7a0a5a1de1fe2a752184efe7b0cd7206f78dc0a1e2538b2b6d`를
   재확인한다. 파일에는 다른 writer의 `Box::pin` 수정이 dirty로 있으므로 해당 hunk를 보존한다.
2. 기존 holder closure의 start/release one-shot을 유지한다. class 정책에는
   `.overflow_policy(OverflowPolicy::Reject)`를 명시해 `max_queue_depth(0)`과 거절 의미를 구분한다.
3. `started_rx`를 bounded await한 뒤 같은 test task에서 첫 `run_blocking` call을 한 번 수행한다.
   결과는 정확히 `RunError::Governor(GovernorError::Rejected(AdmissionVerdict::CpuSaturated { .. }))`
   이어야 한다. rejected closure가 실행되지 않았음도 test-local flag로 확인한다.
4. 그 후 release 신호를 보내고 holder `JoinHandle`을 이름 있는 `bounded` helper로 await한다.
   `await_detached`는 정상 worker result의 lease를 drop한 뒤 반환하므로 이 join이 class capacity
   반환을 증명한다. snapshot에서 inflight/queued 0을 확인한다.
5. 같은 class의 두 번째 `run_blocking` call을 단 한 번 수행하고 정확한 sentinel 값(예: 42)을
   확인한다. 기존 client spawn, saturated/retry channel, 무한 loop, `yield_now`, attempt counter를
   제거한다. holder 자체가 별도 blocking worker이므로 첫 client call의 contention은 유지된다.
6. 각 기다림에 원인별 diagnostic을 붙이고 final `assert_drained(&rt, &["c"])`를 즉시 실행한다.
   assertion panic으로 release sender가 drop되어도 holder의 `blocking_recv`가 반환하는 구조를
   유지한다.

## 테스트와 intentional negative

- positive: holder started → 정확한 `CpuSaturated`/work-not-run → holder completion/slot zero →
  단일 retry 값 42 → exact drain 순서가 고정된다.
- intentional negative: `max_inflight(1)`을 2로 바꾸면 첫 call이 성공하여 typed rejection
  assertion이 실패한다.
- policy negative: queue depth만 바꾸는 mutation은 유효한 negative가 아니다. overflow를
  `QueueWithinDepth`로, depth를 1로 함께 바꾸면 첫 call이 즉시 reject되지 않아 named timeout으로
  실패해야 한다.
- cleanup negative: release sender가 assertion failure로 drop되어도 holder worker가 fixture에
  갇히지 않아야 한다.

## DoD

- SEP22-T03-A01 current unbounded retry/yield/attempt loop와 redundant client channels가 제거된다.
- SEP22-T03-A02 첫 제출의 정확한 `CpuSaturated` 및 work-not-run과 holder 완료 뒤 단일 성공 값이 증명된다.
- SEP22-T03-A03 모든 단계가 bounded이고 holder join 뒤 slot-zero 및 final exact drain이 남는다.
- SEP22-T03-A04 유효한 class/policy negative와 동일 조건 before/after median/worst, executed count가 기록된다.

## 금지되는 임시방편

- retry 횟수, sleep duration, `yield_now` 빈도만 바꾸는 변경.
- release send 직후 holder completion 없이 두 번째 call을 여는 변경.
- 어떤 governor rejection도 허용하는 wildcard assertion.
- queue depth만을 rejection 원인으로 설명하거나 production backoff를 추가하는 변경.
- 첫 제출이 성공해도 최종 성공만으로 통과시키는 변경.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test e2e_scenarios backpressure_retry_makes_forward_progress -- --exact --nocapture
cargo test --locked -p taskmesh --test e2e_scenarios -- --nocapture
```

각 exact filter가 1/1 실행되어야 한다. 동일 host/profile/target policy에서 변경 전후 각 최소
5회 측정하고 source SHA, median, worst를 기록한다. mutation probe는 격리된 임시 worktree와
target에서 수행한다. 이후 integrator가 `just gate`, `just matrix`, `just verify-macos-ci`를
현재 source에서 실행한다.

## Stop/reopen 조건

- holder receipt 이후 첫 call이 `CpuSaturated`가 아니면 assertion을 완화하지 않고 admission
  policy/runtime owner로 재개한다.
- holder completion 후에도 class slot이 nonzero이거나 단일 retry가 실패하면 retry loop를 복원하지
  않고 lease release/forward progress 결함으로 분리한다.
- T04 또는 다른 writer가 `e2e_scenarios.rs`를 먼저 변경하면 owner SHA와 lane 순서를 재고정한다.

## 2026-09-23 구현 및 검증

- class overflow `Reject`를 명시하고, holder start 뒤 첫 submission의 정확한 `CpuSaturated`와
  rejected closure 미실행을 확인한다. Holder release 후 join/slot-zero를 완료한 다음 retry를
  정확히 한 번 수행하여 값 42와 final drain을 확인한다. Client spawn, retry channel, 무한 loop,
  yield 및 attempt counter는 제거했다.
- owner source SHA-256: `e2e_scenarios.rs` =
  `2935d7a27320dff36f068cd7afe598fe55a235185bb54c9a079dc412f6e56974`.
- owner-local rail: `cargo test --locked -p taskmesh --test cancellation_policy --test deadline_cancel --test e2e_scenarios`;
  front door `cargo test`, profile `test`, base HEAD `146233665942d75b73e2b724f781be7e105fd7c4`,
  dirty shared worktree, exit 0. Relevant binary executed/passed: `e2e_scenarios` 9/9;
  failed/filtered/ignored 0. `rustfmt --check --edition 2021` and `git diff --check` passed.
- `max_inflight(1)`을 2로 일시 변경한 exact test는 1/1 실행 후 첫 typed rejection assertion에서
  즉시 실패(exit 101). `QueueWithinDepth`와 depth 1을 함께 적용한 exact test는 1/1 실행 후
  `first submission did not reject` 5초 timeout으로 실패(exit 101). 두 mutation 모두 원복했고
  `e2e_scenarios.rs` SHA-256은 위 owner hash와 같다. 원복 뒤 exact positive 1/1 통과했다.
- 이 시점의 local proof는 non-hermetic이다. 당시에는 before/after timing을 수집하지 않아
  A04가 열려 있었다. exact-source W3 qualification도 아직 없다. 한 차례 full e2e 재실행은 test binary가
  본문 진입 전 macOS loader에서 장시간 대기해 중단했으므로 pass로 계상하지 않는다.

## 2026-09-23 같은 소스의 retry-loop 대조

- `main@1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b`의 동일 production 소스에서
  현재 test 함수와 commit `402cb9d`의 이전 retry-loop 함수만 바꿔 `cargo test --locked
  -p taskmesh --test e2e_scenarios --no-run`의 `test` 프로필로 각각 빌드했다.
  현재 owner SHA-256은 `2935d7a27320dff36f068cd7afe598fe55a235185bb54c9a079dc412f6e56974`,
  대조 함수 삽입 소스는 `3be2906585d8af64b9e63f1c10d5a8d9f46a1106b81a95fbe1dbb832622ccd9b`다.
- exact `backpressure_retry_makes_forward_progress` binary를 같은 macOS 15.6 arm64,
  같은 target directory에서 교차 순서로 각 10회 실행했다. 현재 10/10, 이전 loop
  10/10 통과했고, process wall median/worst는 현재 0.0076/0.2398초, 이전 loop
  0.0070/0.2361초였다. 성공 경로의 유의한 비용 절감은 관찰되지 않았다. 후자의
  worst 값은 두 변형 모두에 나타난 호스트 부하 영향으로 성능 주장의 근거가 아니다.
- 앞 절의 class/policy intentional negatives와 결합해 A01–A04의 오너 로컬 계약을
  충족했다. 임시 소스를 원래 SHA로 복원했고 W3 exact-source full receipt는 별도다.
