# SEP22-T02 cancellation handshakes

- 상태: PLANNED
- finding: TO-02
- priority: P1
- write lane: `cancellation-sequencing`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh/tests/cancellation_policy.rs`, `crates/taskmesh/tests/deadline_cancel.rs`

## 목적

mid-run cancellation 테스트의 wall-clock 추측을 first-poll/spawn handoff로 교체한다. cancel이
실행 전인지 실행 중인지 모호하지 않은 상태에서 Cooperative와 PreSubmitOnly 의미를 검증한다.

## RCA

현재 테스트는 submission을 spawn한 뒤 20~50ms를 기다리면 work가 이미 실행 중일 것이라고
가정한다. 느린 CI에서는 cancel이 pre-submit에 발생할 수 있고 빠른 호스트에서는 불필요한
지연이 된다. stalled executor case도 실제 `CpuExecutor::spawn` 호출 여부를 관측하지 않는다.

## 확정 근거

- Cooperative와 PreSubmitOnly case가 각각 50ms, 20ms sleep으로 mid-run을 추정한다:
  `crates/taskmesh/tests/cancellation_policy.rs:41-98`.
- local cancellation firer는 40ms timer를 사용한다: `crates/taskmesh/tests/deadline_cancel.rs:563-589`.
- stalled CPU case도 spawn 뒤 40ms를 기다려 cancel한다:
  `crates/taskmesh/tests/deadline_cancel.rs:593-660`.

## 목표 불변식

- token 발화 전 work future가 최소 한 번 poll되었거나 executor가 work를 custody했음이 증명된다.
- Cooperative는 mid-run cancel로 `GovernorError::Cancelled`를 반환한다.
- PreSubmitOnly는 같은 시점의 cancel을 무시하고 명시적 release 뒤 원래 값을 반환한다.
- stalled executor 취소 뒤 permit은 queued work가 drop되기 전까지 유지되고 drop 뒤 해제된다.
- 각 handshake, join, drain은 독립적으로 bounded된다.

## 구현 플랜

1. IO/local work에 재사용 가능한 작은 first-poll future를 각 테스트 모듈 내부에 둔다. 첫 poll에
   one-shot을 한 번 보내고 release 신호 전에는 `Pending`을 반환한다.
2. Cooperative case는 first-poll receipt를 bounded await한 뒤 cancel한다. 별도 work release 없이
   cancellation이 종료 원인이어야 한다.
3. PreSubmitOnly case는 first-poll receipt 뒤 cancel하고, 즉시 성공을 기대하지 말고 별도 release를
   보낸 다음 `Ok(42)`를 확인한다.
4. local case는 `poll_fn` 또는 local future 내부에서 first-poll을 알리고, 외부 task가 receipt를
   받은 뒤 cancel하도록 양방향 handshake를 구성한다.
5. `StallExecutor`에 test-local spawn receipt를 추가한다. `spawn`이 work를 `held`에 넣은 뒤 신호를
   보내고, 테스트는 이 receipt 이후에만 cancel한다.
6. 기존 `bounded` helper 또는 명시적 `tokio::time::timeout`을 모든 기다림에 적용한다.
7. terminal reply 직후와 queued work drop 뒤의 accounting assertions를 각각 유지한다.

## 테스트와 intentional negative

- positive: Cooperative IO/local은 first poll 이후 cancel로 종료된다.
- positive control: PreSubmitOnly는 같은 cancel을 무시하고 release 후 42를 반환한다.
- negative: first-poll receipt 전에 token을 발화하도록 mutation하면 mid-run case가 통과하면 안 된다.
- negative: stalled executor의 held work를 지우기 전에 inflight가 0이면 custody invariant 위반이다.
- cleanup negative: release sender가 drop되어도 test가 hang하지 않고 bounded diagnostic을 낸다.

## DoD

- SEP22-T02-A01 대상 네 case에서 sequencing 목적의 20/40/50/80ms sleep이 제거된다.
- SEP22-T02-A02 Cooperative와 PreSubmitOnly가 동일한 first-poll 경계를 기준으로 반대 결과를 증명한다.
- SEP22-T02-A03 stalled executor는 실제 spawn custody receipt 후 취소되고 two-phase accounting을 검증한다.
- SEP22-T02-A04 모든 handshake와 join에 이름 있는 timeout 및 실패 진단이 있다.

## 금지되는 임시방편

- 더 짧은 sleep, 반복 `yield_now`, snapshot busy loop만으로 sequencing을 대체하는 것.
- PreSubmitOnly case의 work를 즉시 완료시켜 cancel과 경쟁시키는 것.
- cancellation policy나 executor 구현을 테스트 편의를 위해 변경하는 것.
- 서로 다른 policy 결과를 하나의 느슨한 accepted set으로 합치는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test cancellation_policy -- --nocapture
cargo test --locked -p taskmesh --test deadline_cancel run_local_honors_cooperative_cancel -- --exact --nocapture
cargo test --locked -p taskmesh --test deadline_cancel run_cpu_escapes_stalled_executor_via_cancel -- --exact --nocapture
```

각 exact filter의 executed count를 기록하며 0개 실행은 검증으로 인정하지 않는다.

## Stop/reopen 조건

- first poll 이후 cancel인데도 typed result가 정책 계약과 다르면 test-only 변경을 중단하고
  cancellation semantic owner로 재개한다.
- test-local executor receipt를 추가할 수 없어 production trait/API 변경이 필요하면 별도 API
  소유 티켓 없이는 진행하지 않는다.
- drain이 bounded window 안에 재현성 있게 끝나지 않으면 timeout을 늘리지 않고 custody leak로
  분류한다.
