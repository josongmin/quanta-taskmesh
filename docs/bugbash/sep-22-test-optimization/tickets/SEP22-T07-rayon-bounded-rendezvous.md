# SEP22-T07 Rayon bounded rendezvous

- 상태: IMPLEMENTED_UNQUALIFIED
- finding: TO-07
- priority: P1
- write lane: `rayon-adapter-test`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh-rayon/tests/rayon_smoke.rs`

## 목적

Rayon adapter smoke test의 channel receive를 bounded rendezvous로 바꿔 worker/pool 결함을 CI
hang 대신 attributable failure로 만든다.

## RCA

테스트는 work를 real pool에 spawn한 뒤 `mpsc::Receiver::recv()`를 무기한 기다린다. worker가
시작하지 않거나 panic/custody 문제가 생기면 test binary 전체가 timeout될 때까지 원인을
제공하지 않는다. 검증하려는 값은 단일 결과이므로 local timeout으로 경계를 둘 수 있다.

## 확정 근거

- 테스트가 2-worker real Rayon executor를 생성하고 closure를 spawn한다:
  `crates/taskmesh-rayon/tests/rayon_smoke.rs:8-18`.
- 결과 assertion은 unbounded `recv()`를 사용한다:
  `crates/taskmesh-rayon/tests/rayon_smoke.rs:19-20`.

## 목표 불변식

- 실제 `RayonCpuExecutor`와 실제 worker 실행을 유지한다.
- 결과는 여전히 정확히 42이고 sender failure도 assertion된다.
- receive wait은 보수적인 local deadline으로 bounded된다.
- timeout은 pool build, work dispatch, result send 중 어느 경계인지 식별 가능한 메시지를 낸다.
- production executor와 topology semantics는 변경하지 않는다.

## 구현 플랜

1. `std::time::Duration`을 import하고 테스트 전용 `RESULT_TIMEOUT` 상수를 둔다.
2. `rx.recv()`를 `rx.recv_timeout(RESULT_TIMEOUT)`으로 교체한다.
3. `expect` 메시지에 worker count와 기다린 결과 의미를 포함하고, timeout/disconnected failure를
   구분할 수 있게 한다.
4. 기존 sender-side `expect`와 `assert_eq!(42)`를 유지한다.
5. test-local negative에서 sender를 drop한 channel이 즉시 `Disconnected`로 끝나는지, 미발화
   sender가 deadline에 `Timeout`을 내는지 검증해 harness 자체를 확인한다.
6. focused binary를 반복 실행해 정상 경로 duration에 deadline이 관여하지 않음을 기록한다.

## 테스트와 intentional negative

- positive: real pool에서 closure가 실행되고 bounded receive가 42를 얻는다.
- negative: closure dispatch를 제거하면 test binary hang 대신 local timeout으로 실패한다.
- negative: sender를 drop하면 disconnected failure가 timeout으로 오분류되지 않는다.
- negative: 잘못된 result는 기존 equality oracle에 잡힌다.

## DoD

- SEP22-T07-A01 모든 channel receive가 bounded이며 unbounded `recv()`가 남지 않는다.
- SEP22-T07-A02 real Rayon pool, worker-count assertion, sender assertion, value oracle이 유지된다.
- SEP22-T07-A03 timeout/disconnect intentional negatives가 harness failure mode를 구분한다.
- SEP22-T07-A04 focused binary의 반복 실행 count와 worst duration이 기록된다.

## 금지되는 임시방편

- Rayon execution을 inline closure 또는 mock executor로 대체하는 것.
- timeout을 workspace/global test configuration에만 맡기는 것.
- receive error를 `Option`/default value로 흡수하는 것.
- production executor에 test-only heartbeat를 추가하는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh-rayon --test rayon_smoke executor_runs_cpu_work -- --exact --nocapture
cargo test --locked -p taskmesh-rayon --test rayon_smoke -- --nocapture
```

정상 경로 worst duration과 선택한 timeout의 배수를 handoff에 기록한다.

## Stop/reopen 조건

- bounded receive가 실제 timeout되면 값을 늘리기 전에 pool creation/dispatch 결함을 adapter owner로
  재개한다.
- 정상 duration이 host class별로 deadline에 근접하면 환경별 adaptive timeout을 넣지 말고 공통
  hang budget 정책을 별도 결정한다.
- 해결에 production executor 변경이 필요하면 이 test-only 티켓을 중단한다.

## 2026-09-23 현재 소스 재검증

- `main@146233665942d75b73e2b724f781be7e105fd7c4`에서 실제 2-worker Rayon pool의
  결과 receive는 `recv_timeout(Duration::from_secs(1))`이며 값 42와 sender-side assertion이
  유지된다. Unbounded `recv()`는 이 owner file에 남지 않았다.
- `cargo test --locked -p taskmesh-rayon --test rayon_smoke`: exit 0, 4/4 실행·통과,
  실패/filtered/ignored 0. Timeout/disconnect intentional negative와 반복 worst duration,
  W3 receipt는 미확보다. 표준 채널 동작만 재검증하는 영구 테스트는 추가하지 않았다.
