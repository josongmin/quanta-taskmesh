# SEP22-T01 capability binding oracle

- 상태: LOCALLY_VERIFIED
- finding: TO-01
- priority: P0
- write lane: `runtime-cancel`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh/tests/runtime_cancel_timeout.rs`

## 목적

blocking 실행의 대기 원인이 class permit인지 blocking capability인지 테스트가 구분해 증명하게
한다. 취소와 acquire timeout이 어느 경계에서 발생했는지 typed verdict로 고정한다.

## RCA

원래 fixture는 class `max_inflight(1)`과 topology `blocking_threads(1)`을 동시에 한 칸으로
제한한다. holder가 시작된 뒤 waiter를 넣으면 두 자원이 모두 포화되므로 governor의
단일 queue에서 어떤 capacity가 blocker인지 관찰할 수 없다. 따라서 테스트 이름은 substrate wait를
주장하지만 class admission wait만 검증해도 통과할 수 있다.

2026-09-23 review에서 첫 수정의 class timeout control도 blocking spec을 직접 admit하여
class와 blocking capability를 함께 점유한 것으로 확인했다. 그 control의 class verdict는
두 blocker가 공존할 때 class를 우선하는 resolver 순서에 의존했다. cancellation verdict도
두 경로에서 동일하므로 capacity 원인의 paired oracle이 될 수 없다.

## 확정 근거 (수정 전 baseline)

- 단일 fixture가 두 용량을 모두 1로 설정한다: `crates/taskmesh/tests/runtime_cancel_timeout.rs:10-23`.
- 문제 테스트는 holder 시작만 확인하고 waiter를 제출한 뒤 20ms를 기다려 취소한다:
  `crates/taskmesh/tests/runtime_cancel_timeout.rs:126-176`.
- 기존 pre-submit 우선순위 테스트는 별도로 존재하므로 이 티켓은 그 의미를 중복하지 않는다:
  `crates/taskmesh/tests/runtime_cancel_timeout.rs:82-124`.

## 목표 불변식

- substrate-bound fixture는 class capacity를 남겨 두고 blocking capability만 점유한다.
- capability wait의 timeout은 substrate-specific typed rejection이어야 한다.
- class-bound control은 동일한 substrate 여유 아래 class acquire verdict를 내야 한다.
- 이미 취소된 요청은 어느 capacity wait보다 먼저 `CancelledBeforeSubmit`으로 종료한다.
- 모든 holder와 waiter는 bounded join을 가지며 종료 후 class accounting은 0이다.

## 구현 플랜

1. 기존 공용 fixture를 의미별 helper로 분리한다. substrate fixture는
   `max_inflight(2)`, `blocking_threads(1)`로 구성하고 class fixture는 class 한도를 1로 둔다.
2. substrate holder는 blocking closure 안에서 `started`를 보내고 명시적 release 신호까지
   capability를 유지한다.
3. capability waiter가 governor queue에 들어간 것을 bounded snapshot으로 확인한다.
   그때 class inflight는 1/2, blocking role과 physical domain은 1/1이어야 한다.
   같은 class의 IO zero-budget 요청을 성공시켜 class 여유를 실행 경로로 독립 검증한다.
4. capability가 점유된 상태에서 acquire timeout을 발생시키고 정확한 substrate verdict를
   assert한다.
5. paired control에서 class만 포화시키고 blocking capability는 비워 둔 채 class acquire
   timeout verdict를 assert한다.
6. 기존 cancellation case는 arbitrary sleep을 제거하고 waiter가 의도한 wait 경계에 도달한
   뒤 token을 발화시킨다. 공개 API로 직접 관측할 수 없다면 snapshot 기반 bounded helper를
   사용하고, 생산 코드에 test hook을 추가하지 않는다.
7. 각 case 종료 시 holder release, join, exact drain을 확인한다.

## 테스트와 intentional negative

- positive: class capacity가 남아 있고 blocking capability만 점유된 요청이 release 후 성공한다.
- negative: 같은 상태에서 zero-budget acquire는 class timeout으로 뭉개지지 않고
  substrate-specific verdict를 반환한다.
- control negative: class slot만 포화된 요청은 substrate verdict를 반환하면 실패한다.
- precedence negative: pre-cancelled token이 timeout/capacity verdict로 반환되면 실패한다.
- mutation probe: substrate fixture의 `max_inflight(2)`를 1로 되돌렸을 때 paired oracle 중
  적어도 하나가 실패해야 한다.

## DoD

- SEP22-T01-A01 substrate와 class capacity가 독립된 fixture 및 paired verdict oracle이 있다.
- SEP22-T01-A02 cancellation은 관측된 wait 경계 뒤 발화하며 임의 지연에 의존하지 않는다.
- SEP22-T01-A03 모든 비정상 경로가 bounded이고 holder release 및 exact drain을 검증한다.
- SEP22-T01-A04 mutation probe 결과와 focused test 명령/exit code가 handoff에 기록된다.

## 금지되는 임시방편

- sleep 시간을 늘리거나 retry/yield 횟수를 늘리는 변경.
- 여러 typed verdict를 `matches!` OR로 허용하는 assertion.
- production semaphore 순서를 테스트 편의를 위해 변경하거나 test-only hook을 추가하는 변경.
- substrate와 class contention을 같은 fixture에서 다시 동시에 만드는 변경.

## 검증 명령

```sh
cargo test --locked -p taskmesh --test runtime_cancel_timeout -- --nocapture
cargo test --locked -p taskmesh --test runtime_cancel_timeout cancel_while_waiting_for_substrate_rejects_without_waiting_for_capacity_v1 -- --exact --nocapture
cargo test --locked -p taskmesh --test runtime_cancel_timeout cancel_while_queued_for_governor_abandons_ticket_promptly_v1 -- --exact --nocapture
```

두 번째/세 번째 명령은 exact filter로 각각 1개 테스트가 실행되어야 한다.

## Stop/reopen 조건

- 공개 snapshot으로 class admission과 substrate wait를 분리 관측할 수 없고 production
  instrumentation이 필요하면 중단하고 runtime owner 티켓으로 재개한다.
- paired control이 현재 공개 계약과 다른 verdict를 증명하면 테스트를 완화하지 말고 정책
  문서와 runtime owner를 함께 재검토한다.
- 다른 변경으로 HEAD/tree가 바뀌거나 소유 경로가 dirty이면 baseline을 다시 고정한다.

## 2026-09-23 focused handoff

- 시작/종료 baseline: `main@146233665942d75b73e2b724f781be7e105fd7c4`, tree
  `d3a1480962a8a148a6efac3a77ff6cd185bcb447`. 작업 전 dirty 경로는
  `crates/taskmesh/tests/deadline_cancel.rs`, `crates/taskmesh/tests/e2e_scenarios.rs`였고
  `docs/bugbash/sep-22-test-optimization/review-prompt.md`는 untracked였다. 이 handoff의
  runtime owner 입력은 `runtime_cancel_timeout.rs` SHA-256
  `8213e8710455edd6855f4324533a455d512e012715048afa95341b26e1e5bdc5`이다.
- A01: class-only control은 IO permit으로 class를 채우고 blocking role/physical domain
  사용량 0을 확인한다. capability control은 blocking role/physical domain 사용량 1과
  class 여유를 IO zero-budget 성공으로 확인한다. 두 zero-budget timeout은 서로 다른
  정확한 typed verdict를 요구한다. 별도 positive case는 holder release 후 queued waiter가
  실제 실행됨을 확인한다.
- A02: capability waiter가 governor queue에 들어간 snapshot을 bounded polling으로
  관측하고 blocking role/physical domain 점유와 같은 class의 IO zero-budget 실행으로
  capability 포화·class 여유를 독립 확인한 뒤 cancel한다.
  holder는 release 신호 전까지 capability를 유지한다. release 후 승격 positive case도
  동일한 class 여유를 확인한다.
- A03: T01 holder/waiter의 start/join은 명시적 timeout으로 제한하고, channel sender가
  assertion panic으로 drop되어도 worker가 빠져나온다. release 뒤 class queue/inflight와
  blocking role/physical domain 사용량 0을 확인한다.
- A04: `cargo test --locked -p taskmesh --test runtime_cancel_timeout` exit 0,
  selected/executed 10/10. `cancel_while_waiting_for_substrate_rejects_without_waiting_for_capacity_v1`
  exact filter 5회 반복은 각각 1/1, exit 0이었다. 위 SHA-256과 동일한 test file을
  임시 detached worktree에 복사한 뒤 `substrate_contention_runtime`의
  `max_inflight(2)`만 `1`로 바꿨다. 명령은
  `cargo test --locked -p taskmesh --test runtime_cancel_timeout zero_budget_projects_the_current_capability_blocker_without_running_work -- --exact --nocapture`;
  selected/executed 1/1, exit 101, 실패 이유는 `spare-class-probe`가
  `PermitAcquireTimedOut`으로 거절된 것이다. hardening 후 같은 단일 fixture mutation에서
  cancellation exact test도 selected/executed 1/1, exit 101로 실패했으며 동일한
  `spare-class-probe` 거절을 보고했다. 이 mutant는 별도 target directory에서 빌드했다.
- 검증 단계는 focused local이다. `cargo fmt -p taskmesh -- --check`, `just semgrep`
  (133 files, 0 findings), `just clippy` (test, production, rayon, docs, bench)는 exit 0이다.
  Rust 1.95의 `clippy::map_err_ignore`가 탐지한 `crates/taskmesh/src/builder.rs:315`의
  익명 변환 오류 바인딩을 의도가 드러나는 이름으로 바꾸어 gate를 통과시켰다.
  `just verify-macos-full` exact-source W3 receipt는 아직 없다. 이 ticket의
  `LOCALLY_VERIFIED`는 campaign qualification을 뜻하지 않는다.
