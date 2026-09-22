# SEP22-T03 backpressure handshake

- 상태: PLANNED
- finding: TO-03
- priority: P1
- write lane: `e2e-scenarios`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh/tests/e2e_scenarios.rs`

## 목적

backpressure forward-progress 시나리오를 정확히 한 번의 saturation 관측과 한 번의 release 후
성공으로 구성한다. 최대 1000회 retry와 고정 sleep을 제거하면서 typed rejection과 progress를
동시에 보존한다.

## RCA

holder 시작을 50ms sleep으로 추정하고 client는 5ms 간격으로 최대 1000회 재시도한다. release도
추가 80ms 후 실행된다. 이 구조는 scheduler 속도에 따라 최초 시도가 성공할 수 있고, 실패 시
최대 5초 이상의 tail을 만든다. `attempts > 0`은 첫 결과가 정확히 `CpuSaturated`였다는 oracle도
아니다.

## 확정 근거

- class는 1-slot, queue depth 0으로 즉시 `CpuSaturated`를 내도록 구성된다:
  `crates/taskmesh/tests/e2e_scenarios.rs:260-271`.
- holder/client 순서는 50ms sleep, 1000회 loop, 회당 5ms sleep, 80ms release에 의존한다:
  `crates/taskmesh/tests/e2e_scenarios.rs:273-307`.
- 최종 assertion은 성공 attempt가 0보다 큰지만 확인한다:
  `crates/taskmesh/tests/e2e_scenarios.rs:309-312`.

## 목표 불변식

- 첫 client 제출 전 holder가 실제 blocking closure에 진입해 유일한 slot을 점유한다.
- 첫 client 제출은 정확히 `CpuSaturated`로 실패한다.
- saturation receipt 이후에만 holder를 release한다.
- release 후 두 번째 제출은 한 번에 성공하며 반환값과 exact drain을 검증한다.
- 전체 프로토콜은 한 개의 이름 있는 outer timeout으로 hang을 제한한다.

## 구현 플랜

1. holder closure에 `started_tx`를 추가하고 release receiver를 유지한다.
2. 테스트 본문은 bounded `started_rx`를 받은 뒤 첫 client call을 직접 실행한다.
3. 첫 결과를 `RunError::Governor(GovernorError::Rejected(AdmissionVerdict::CpuSaturated { .. }))`
   하나로 정확히 assert한다.
4. rejection을 관측한 뒤 release를 보내고 holder를 join한다.
5. 동일 class에 두 번째 call을 한 번 실행하여 결과 값을 assert한다. retry loop는 만들지 않는다.
6. 전체 async protocol을 bounded helper로 감싸되 각 channel failure가 원인을 드러내는 메시지를
   갖게 한다.
7. 종료 시 `assert_drained`를 즉시 호출한다. drain 문제는 T04가 다루므로 settle sleep을 넣지 않는다.

## 테스트와 intentional negative

- positive: start → saturation → release → one-shot success 순서가 고정된다.
- negative: holder 시작 전에 첫 client를 실행하는 mutation은 saturation assertion을 실패시켜야 한다.
- negative: `max_queue_depth(0)`을 queueable 값으로 바꾸면 typed verdict assertion이 실패해야 한다.
- cleanup negative: holder release를 누락하면 outer timeout이 이름 있는 실패를 반환해야 한다.

## DoD

- SEP22-T03-A01 시나리오에서 50/80/5ms sleep과 1000회 retry loop가 제거된다.
- SEP22-T03-A02 첫 제출의 정확한 `CpuSaturated`와 release 후 단일 성공을 모두 검증한다.
- SEP22-T03-A03 holder start/release/join 및 전체 protocol이 bounded이고 exact drain이 남는다.
- SEP22-T03-A04 변경 전후 동일 명령의 median/worst timing과 executed count가 기록된다.

## 금지되는 임시방편

- retry 횟수나 sleep duration만 축소하는 변경.
- 어떤 governor rejection도 허용하는 wildcard assertion.
- queue depth를 늘려 테스트를 통과시키거나 production backoff를 추가하는 변경.
- 첫 제출이 성공해도 최종 성공만으로 통과시키는 변경.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test e2e_scenarios backpressure_retry_makes_forward_progress -- --exact --nocapture
cargo test --locked -p taskmesh --test e2e_scenarios -- --nocapture
```

동일 target/profile에서 최소 5회 측정하고 source SHA가 같은 run만 비교한다.

## Stop/reopen 조건

- holder receipt 이후 첫 call이 `CpuSaturated`가 아니면 assertion을 완화하지 않고 admission
  policy/runtime owner로 재개한다.
- deterministic protocol에서도 release 후 단일 call이 실패하면 retry를 복원하지 않고 forward
  progress 결함으로 분리한다.
- T04 또는 다른 writer가 `e2e_scenarios.rs`를 먼저 변경하면 merge하지 말고 lane 순서를 재고정한다.
