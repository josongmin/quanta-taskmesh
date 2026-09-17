# TM16-040 — Mixed-substrate soak가 모든 실행 오류를 버려 360건 전부 거부되어도 통과한다

- Severity: P2
- Status: OPEN / source-extracted controlled mutation reproduction
- Lane: R — host integration test assertions; G — test-quality coordination
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh/tests/e2e_chaos.rs:104-198`: `mixed_substrate_soak_with_concurrent_sweeper`가 IO/CPU/blocking 혼합 360건을 제출한다.
- 같은 파일 `:172-185`: 세 run 결과 모두 `let _ = ...await`로 버린다.
- 같은 파일 `:190-197`: join panic 여부, sweeps>0, 최종 zero ledger만 검증한다. 실제 작업 시작/완료, 성공 수, 허용하는 오류 범주를 확인하지 않는다.
- `crates/taskmesh-engine/src/features/admission/mod.rs:80-81`: disabled class는 job 실행 없이 `ClassDisabled`를 반환한다.
- [Retained source-extraction harness](evidence/repro/build.rs), [observation](evidence/repro/src/lib.rs): `mixed_soak_assertions_pass_with_every_submission_rejected`.

## Trigger / 관찰

production source/test 파일은 수정하지 않았다. Build script는 실제 test helpers와 해당 async function을 읽어 Cargo OUT_DIR에 두 버전을 생성한다.

1. 함수 이름만 바꾼 unmodified control: 기존 soak body/assertions 그대로 실행.
2. mutation: 세 class의 `max_inflight(8/2/1)`만 모두 0으로 변경한다. 원래 assertion을 모두 보존하면서, 이전에 버리던 각 결과가 정확히 `ClassDisabled`임을 추가로 단언한다.

둘 다 통과한다. mutant의 360건은 모든 substrate에서 job 실행 전에 거부되지만 sweeper가 돌고 ledger가 처음부터 0이라 기존 assertion이 green이다. Source guard로 정확히 세 policy 값과 세 run-result branch만 변환되도록 고정했다.

## 원인 / 영향 / 범위

`JoinHandle` 성공은 내부 `run_*`의 성공이 아니다. 반환된 RunError를 버리고 terminal zero state만 확인하면 실제 governor/worker 전이를 전혀 거치지 않은 상태도 operational soak의 성공처럼 보인다. 이 테스트가 특정 오류·fixture drift·admission regression을 놓칠 수 있다는 증거다.

현재 정상 runtime이 실제로 360건을 거부한다는 주장이나 전체 workspace가 이 mutation에도 green이라는 주장은 아니다. 기존 전체 suite의 다른 테스트는 검출할 수 있다. 재현은 해당 test body의 assertion sufficiency에 한정하며 source-extracted fixture mutation이지 production mutation campaign은 아니다. TM16-017 scanner enrollment와 달리 테스트 자체의 observable verdict 검증 누락이다.

## 보완 계획

- per-substrate attempted/started/completed/success 및 rejected/runtime-error/task-error를 분리 계수한다. 현재 fixture가 전부 성공을 의도한다면 run 결과를 unwrap/정확한 값으로 검증한다.
- overload를 허용하는 soak라면 허용 verdict를 enum으로 제한하고, 각 substrate에 최소 실제 작업 수와 live inflight 관측을 요구한다.
- sweeps>0만 아니라 live work와 실제 overlap을 동기화된 marker로 검증한다. 고정 sleep만 추가하지 않는다.
- 종료 시 zero accounting 검증은 유지한다. error count와 작업 보존식을 함께 검증한다.

## Acceptance / 회귀 검증

- 현재 정상 fixture는 각 substrate에서 실제 작업을 실행하고 기대 결과 수를 만족한다.
- retained all-disabled mutation은 repaired original assertions에서 실패해야 한다.
- CPU executor result loss/runtime error를 주입해도 silently green이 되지 않는다.
- cancellation/overload가 의도된 별도 시나리오는 typed expected outcomes와 최소 성공 실행을 구분한다.
