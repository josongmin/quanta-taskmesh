# SEP22-T04 drain completion boundary

- 상태: IMPLEMENTED_UNQUALIFIED
- finding: TO-04
- priority: P2
- write lane: `e2e-scenarios`
- 선행 티켓: SEP22-T03
- 소유 경로: `crates/taskmesh/tests/e2e_scenarios.rs`, `crates/taskmesh/tests/e2e_chaos.rs`

## 목적

모든 제출 handle이 terminal result를 반환한 뒤 exact drain이 즉시 성립하는지 검증하고, 의미 없는
post-completion settle sleep을 제거한다.

## RCA

두 E2E 시나리오는 모든 handle을 join하고 conservation을 확인한 뒤 50~60ms를 추가로 잔다.
이 sleep은 상태 전이를 유발하지 않으며 accounting release가 terminal reply보다 늦어도 테스트를
통과시킨다. 실제 completion 계약 위반을 숨기면서 suite 시간만 늘린다.

## 확정 근거

- overload case는 모든 handle join과 64개 결과 conservation 뒤 60ms를 기다린다:
  `crates/taskmesh/tests/e2e_scenarios.rs:358-366`.
- chaos race case도 200개 반환 확인 뒤 50ms를 기다린다:
  `crates/taskmesh/tests/e2e_chaos.rs:354-366`.
- 두 위치 모두 sleep 직후 동일한 `assert_drained`를 실행한다.

## 목표 불변식

- join 완료는 해당 요청의 class accounting release까지 포함하는 관측 가능한 completion boundary다.
- 반환 수 conservation과 overload shedding oracle은 유지된다.
- final snapshot은 polling 없이 exact zero를 보여야 한다.
- runtime이 terminal reply 이전 drain을 계약하지 않는 예외 경로라면 그 사실을 명시적으로 분리한다.

## 구현 플랜

1. T03 변경이 정착된 HEAD에서 시작하고 `e2e_scenarios.rs` 충돌 여부를 다시 확인한다.
2. overload case의 60ms와 chaos case의 50ms post-join sleep만 제거한다.
3. `assert_drained`를 join/conservation assertion 바로 다음 줄에서 실행한다.
4. focused test를 반복 실행하여 instant drain이 안정적으로 성립하는지 확인한다.
5. 실패하면 snapshot의 inflight/queued/used resources와 terminal reply 순서를 캡처한다. polling
   helper나 grace period를 추가하지 않는다.
6. completion-before-release가 의도된 production 계약으로 확인될 경우 테스트 최적화를 중단하고
   owner 문서/typed completion boundary 티켓을 별도로 연다.

## 테스트와 intentional negative

- positive: 모든 join 이후 첫 snapshot이 exact drain이다.
- negative: 마지막 permit release를 terminal reply 뒤로 미루는 mutation은 즉시 drain assertion에
  잡혀야 한다.
- conservation negative: 반환 count를 줄이거나 join을 생략하면 drain만으로 통과하면 안 된다.
- chaos case는 final usability check를 그대로 유지하여 drain 뒤 runtime 재사용도 증명한다.

## DoD

- SEP22-T04-A01 두 post-completion sleep이 제거되고 즉시 exact drain assertion이 실행된다.
- SEP22-T04-A02 기존 result conservation, shed, typed error, final usability oracle이 유지된다.
- SEP22-T04-A03 focused 반복 실행에서 flake가 없고 before/after median/worst가 기록된다.
- SEP22-T04-A04 instant drain 실패 시 우회 없이 product-owner 증거로 전환하는 handoff가 있다.

## 금지되는 임시방편

- sleep을 polling, retry, `yield_now` loop로 바꾸는 것.
- drain assertion을 eventually-zero 또는 upper bound로 약화하는 것.
- join하지 않은 task가 남은 상태에서 snapshot만 검사하는 것.
- T03과 병렬로 같은 파일을 편집하는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test e2e_scenarios overload_is_bounded_and_fail_closed -- --exact --nocapture
cargo test --locked -p taskmesh --test e2e_chaos -- --nocapture
cargo test --locked -p taskmesh --test e2e_scenarios --test e2e_chaos
```

각 focused case를 같은 host/target에서 최소 20회 실행해 flake 여부를 기록하되 한 번의 실패도
평균으로 상쇄하지 않는다.

## Stop/reopen 조건

- terminal reply 뒤 snapshot이 non-zero이면 sleep을 되돌리기 전에 production completion 계약과
  permit custody를 조사하는 별도 티켓으로 재개한다.
- T03 미완료 또는 동일 파일 dirty 상태면 이 티켓을 시작하지 않는다.
- 호스트 부하로 실행 자체가 지연된 경우 timing evidence는 폐기하되 semantic failure는 폐기하지 않는다.

## 2026-09-23 현재 소스 재검증

- `main@146233665942d75b73e2b724f781be7e105fd7c4`에는 두 post-join sleep이 이미 없다.
  두 test는 모든 handle join과 결과 수 conservation 직후 `assert_drained`를 즉시 호출한다.
- `cargo test --locked -p taskmesh --test e2e_scenarios --test e2e_chaos`: exit 0,
  `e2e_scenarios` 9/9 및 `e2e_chaos` 3/3 실행·통과, 실패/filtered/ignored 0.
- `overload_is_bounded_and_fail_closed`와 `claim_timeout_abandon_race_storm`을 각각 동일한
  로컬 소스에서 20회 반복해 20/20 통과했다. Semantic 안정성 증거이며, 동시 호스트 부하가
  있어 timing 비교에는 사용하지 않는다. 변경 전후 median/worst와 W3 receipt는 미확보다.
