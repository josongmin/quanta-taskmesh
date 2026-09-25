# S25-007 — stage 선언과 child 수명

- 상태: PLANNED. 우선: P1. 선행: [S25-001](S25-001-contract-boundaries.md), [S25-006](S25-006-deadline-and-custody.md). 소유: host/runtime owner; 공개 API 문서는 API owner.
- 주 담당 시나리오: A11, H21, H35.

## 목적

plan 선언, 한 번의 host dispatch, caller-owned root future, raw Tokio child의 수명 경계를 정확하게 고정한다. `TaskSpec.stages`/`reduce_stage`는 검증 대상 선언이고 `run_*`는 첫 dispatch만 실행한다. direct Governor admission은 선언 stage들의 capability를 예약할 수 있지만 host는 실제 dispatch의 role/physical pool만 예약한다. `run_io`/`run_local` root lease는 caller future가 소유한다. `run_local`은 호출별 `LocalSet::run_until(root)`을 사용하므로 await하지 않은 local child 완료를 약속하지 않는다. raw `tokio::spawn` child는 host 추적 범위 밖이다.

## 변경 파일

| 구분 | 경로 | 변경 목적 |
|---|---|---|
| 기존 구현, 확인 우선 | `crates/taskmesh/src/runtime.rs`, `crates/taskmesh/src/execution_plan.rs` | first-dispatch requirement, IO/local RAII lease, `LocalSet::run_until` 소유 경계. 반례가 없으면 구현 변경 없음. |
| 기존 계약 | `crates/taskmesh-contract/src/task.rs` | stage/reduce 선언과 child scope builder의 공개 설명; 실행 계약 변경은 contract/API owner만 적용. |
| 기존 fixture | `crates/taskmesh/tests/hardening_dispatch_resolution.rs`, `runtime_local.rs`, `local_runtime_guard.rs`, `e2e_scenarios.rs` | direct vs host reservation, !Send root, wrong substrate, stage 비자동 실행. |
| 신규 후보 | `crates/taskmesh/tests/hardening_root_child_scope.rs` | H21/H35의 caller drop/panic, local/ambient child 생존·drop, drain 경계 격리. |
| 계약 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | 자동 DAG·reducer·detached child 추적 범위 명시. 필요한 경우만 별도 structured-child RFC 작성. |

## 구현 순서

1. A11에서 plan validation과 실제 host dispatch를 별도 계수한다. 두-stage와 deterministic reduce policy를 선언하고 첫 stage만 `run_*`에 제출한다. 후속 stage/reducer closure에 접근하지 않았는지, 직접 Governor admission과 host admission의 예약 범위가 다른지 검증한다.
2. H21에서 IO/local root를 caller-owned 상태로 barrier에 묶고 abort/drop 및 panic을 각각 주입한다. root lease drop, 다음 admission, raw ambient child 독립 실행을 계수한다. root panic은 caller task에서 관측하되 detached worker의 `WorkerPanicked`로 포장하지 않는다.
3. H35에서 `run_local` 내부 `spawn_local` child와 raw `tokio::spawn` child를 별도로 시작한다. root가 await하지 않고 반환할 때 local child의 drop/미완료와 ambient child의 계속 실행을 독립 channel로 확인한다. drain `Ok`를 raw child 종료 증거로 쓰지 않는다.
4. 결과가 현행 계약과 같으면 문서와 fixture만 보강한다. raw child까지 구조적으로 추적해야 한다는 제품 요구가 생기면 현재 root lease를 암묵적으로 연장하지 말고 child registration/ownership/cancellation API, capacity reservation, drop/drain semantics를 별도 RFC로 설계한다.

## DoD

- [ ] `S25-007-A11`: stage/reduce 선언만으로 후속 closure 실행 0, 신규 permit 0, pool charge 증가 0. caller가 별도 `run_*`로 제출한 후속 작업에만 각자 새 permit이 생긴다. direct Governor의 전체 선언 예약과 host의 실제 dispatch 예약을 서로 다른 control로 확인한다.
- [ ] `S25-007-H21`: IO/local root의 caller abort/drop과 panic에서 root lease는 정확히 한 번 반환되고 후속 admission이 진전한다. root panic은 caller task panic이며 `WorkerPanicked` 변환이 없다. raw `tokio::spawn` child는 root 뒤에도 계속 실행할 수 있고 drain이 이를 기다리지 않음을 child barrier와 별도 counter로 보인다.
- [ ] `S25-007-H35`: root가 await하지 않은 `spawn_local` child의 미완료/drop과 ambient Tokio child의 독립 진전을 분리한다. root 결과와 drain 결과, 각 child 완료 여부를 별도 채널로 기록한다. local child가 실행되거나 종료될 시각을 scheduler 운에 맡긴 fixture는 불합격.

## 계획된 검증

구현 시 owner-local: `cargo test --locked -p taskmesh --test hardening_dispatch_resolution`, `cargo test --locked -p taskmesh --test runtime_local`, `cargo test --locked -p taskmesh --test local_runtime_guard`, `cargo test --locked -p taskmesh --test e2e_scenarios`; 신규 binary면 그 `--test` 추가. 공개 문구를 바꾸면 `just doctest`와 `just rustdoc`도 선택한다. 이번 계획 작성에서 테스트는 실행하지 않는다.

## 인계·중단 조건

S25-012에 default/feature/doctest 선택을 넘긴다. S25-006의 requested-stack owned runtime child는 raw IO/local child와 계약이 다르므로 같은 lease 규칙으로 일반화하지 않는다. raw child까지 자동 추적하라는 요구가 나오면 현재 티켓의 scope를 넘으며 API owner와 structured-child RFC를 먼저 결정한다.
