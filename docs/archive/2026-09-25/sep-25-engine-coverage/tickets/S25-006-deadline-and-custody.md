# S25-006 — deadline 응답과 worker custody

- 상태: PLANNED. 우선: **P0 H34**, 나머지 P1. 선행: [S25-001](S25-001-contract-boundaries.md), [S25-005](S25-005-executor-authority.md). 소유: host/runtime owner; engine lease 변경은 engine owner.
- 주 담당 시나리오: D05, D15, H11, H12, H19, H20, H31, H34.

## 목적

caller 응답, root 실행 종료, owned runtime teardown, 마지막 worker/child 종료, Governor lease 반환을 독립 사건으로 모델링한다. 현재 `runtime.rs::run_async_with_requested_stack_v1`은 정상 `Completed`를 runtime drop 뒤 전송하고 deadline/panic `Terminated`는 teardown 전에 전송한다. 정상 root가 기한 전에 끝나도 `spawn_blocking` child teardown으로 응답이 늦어지는 H34 반례가 가능하다. 기존 `hardening_deadline_custody.rs`에는 deadline/panic 사례만 있다.

Accepted ADR 0003의 `RunFor`는 worker-start 기준 **실행 예산**이다. root가 예산 안에 끝나면 cleanup이 길다는 이유로 late `Ok`를 deadline 오류로 바꾸지 않는다. blocking `RunFor`는 caller wait만 제한한다. S25-001 D-S25-06에서 `CompleteBy`가 caller 응답까지 포함한다고 **확정한 뒤** H34 수정에 착수한다. 이는 아직 현행 보장이 아니다. 정상 성공의 release fence는 유지하고, 기한에 terminal을 보낼 때는 lease를 반환하지 않는다.

## 변경 파일

| 구분 | 경로 | 변경 목적 |
|---|---|---|
| 기존 구현 | `crates/taskmesh/src/runtime.rs` | `run_async_with_requested_stack_v1` outcome/response arbiter, `await_detached`, `AcquisitionArbiter`, `ExecutionLease`의 상태·응답·custody 전이. 요청당 bounded completion channel/cell. |
| 기존 구현, 조건부 | `crates/taskmesh/src/execution_plan.rs` | stack/deadline preflight와 `ValidatedDispatchPlan` 확인; 경계 오류가 재현될 때만 적용. |
| 기존 engine, 조건부 | `crates/taskmesh-engine/src/engine/governor.rs`, `engine/state.rs` | host lease owner로 설명할 수 없는 phase/settlement 오류가 재현된 경우 engine owner에게 이관. |
| 기존 fixture | `crates/taskmesh/tests/hardening_deadline_custody.rs`, `runtime_cpu_executor.rs`, `hardening_executor_protocol.rs`, `hardening_drain.rs`, `deadline_cancel.rs` | 정상 root+live child, accepted closure, external lease 선점, clock/tie 경계. |
| 신규 후보 | `crates/taskmesh/tests/hardening_deadline_response.rs` | H34 timeline이 기존 파일을 과도하게 키우면 격리. |
| 계약 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | `RunFor`·`CompleteBy`, response·custody의 각 보장. Accepted ADR 0003은 과거 결정으로 유지. |

## 구현 순서

1. producer/root started, root result ready, caller response, child still running, teardown finished, permit release를 barrier/channel/atomic ledger로 별도 기록한다. snapshot phase는 보조 관측이고 독립 oracle 원본은 아니다.
2. S25-001의 응답 범위 결정 뒤, requested-stack worker가 root 완료 후 cleanup 중에도 absolute timer가 caller에게 **한 번만** terminal을 전달하도록 response owner를 분리한다. terminal 후 late `Ok`/task error를 버리되 worker/lease는 유지한다. cleanup이 기한 내 끝난 정상 결과만 release fence 후 전달한다.
3. `RunFor`는 root의 worker-start/finish timestamp로 판정한다. `CompleteBy` absolute clock, acquire timeout checked budget, `RunFor` start clock을 합치지 않는다. blocking/CPU sync에는 `CompleteBy`를 확대하지 않는다.
4. accepted sync CPU/blocking/dedicated worker는 caller drop/cancel/timeout 뒤에도 closure가 lease를 소유하는지 확인한다. direct Governor token이 host phase를 선점했으면 user closure를 시작하지 않는다.
5. terminal 단일성, cleanup 뒤 refund, drain wake 순서를 확인한다. `shutdown_timeout` 반환은 child 종료 증거가 아니다.

## DoD

- [ ] `S25-006-D05`: ZERO acquire는 free capacity에 즉시 admit; deadline equality는 expired. IO/local/requested-stack async `RunFor` checked-add overflow는 typed `PolicyViolation`과 lease 반환. started sync detached `RunFor` timer overflow는 그 timer만 unbounded이고 worker 종료까지 charged. 시작 여부를 함께 판정.
- [ ] `S25-006-D15`: stack 0/1/MAX/MAX+1/target `usize` overflow를 blocking·CPU·requested-stack async별로 판정한다. 0/상한 초과/변환 불가는 typed preflight 오류와 OS spawn 0; 숫자상 허용값은 OS가 거부할 수 있으며 그때 typed `WorkerUnavailable`·refund. MAX 실제 OS spawn 성공을 강요하지 않는다.
- [ ] `S25-006-H11`: pre-submit cancel, absolute deadline, acquire timeout의 동일 tick/순차 tie에서 cancel→absolute→relative precedence, equality expired. admission 직후 만료는 permit 반환 뒤 typed 오류, closure 시작 0, queue/ticket 0. 주입 clock/barrier 사용.
- [ ] `S25-006-H12`: started blocking/CPU/dedicated sync worker의 deadline/cancel/caller drop 후 second admission은 bound, drain은 `NotDrained`; worker 종료 후에만 후속 admission·drain 완료. 외부 active counter와 snapshot을 대조.
- [ ] `S25-006-H19`: accepted closure가 살아 있는 동안 `RunFor` 미시작, caller cancel/drop 후에도 lease·slot 유지, second admission bound, drain `NotDrained`; 실제 execute/drop 뒤 정산. 조기 `JobAbandoned` 금지.
- [ ] `S25-006-H20`: requested-stack deadline/panic terminal 응답이 live blocking child teardown보다 앞선다. deadline은 `cleanup_pending`, panic은 `running` phase일 수 있으나 child 종료 전 dedicated slot 재판매 0·drain `Ok` 0, 종료 후 refund 1.
- [ ] `S25-006-H31`: direct `governor().advance_phase`가 host permit을 먼저 lease하면 host work 0, typed `PolicyViolation`, host drop으로 external token refund 0; `release_leased(token)` 전 drain `NotDrained`, 이후 drain 완료.
- [ ] `S25-006-H34`: 정상 root가 `RunFor`/`CompleteBy` 전에 끝나고 `spawn_blocking` child가 barrier에 남는다. `RunFor`는 cleanup 뒤 늦은 성공·release fence를 허용한다. D-S25-06 확정 시 `CompleteBy`는 absolute 기한 내 terminal 1회, 늦은 `Ok` 없음, 당시 dedicated lease 보유, child 종료 뒤 refund/drain 완료. 다른 결정이면 소비자 영향과 fixture 기대를 먼저 갱신.

## 계획된 검증

구현 시 owner-local: `cargo test --locked -p taskmesh --test hardening_deadline_custody`, `cargo test --locked -p taskmesh --test hardening_drain`, `cargo test --locked -p taskmesh --test runtime_cpu_executor`, `cargo test --locked -p taskmesh --test deadline_cancel`; 신규 binary면 그 `--test` 추가. fixed sleep만으로 순서를 증명하지 않고 불가피한 시간 assertion에는 여유와 상한을 분리한다. 이번 계획 작성에서 테스트는 실행하지 않는다.

## 인계·중단 조건

S25-007에 raw child 추적 범위, S25-011에 응답과 custody의 별도 모집단, S25-012에 gate selection을 넘긴다. cleanup 전에 lease가 풀리거나 terminal 뒤 정상 성공이 다시 관측되면 중단한다. `CompleteBy` 재정의가 breaking이면 S25-001에서 migration/버전 계약을 먼저 확정한다.
