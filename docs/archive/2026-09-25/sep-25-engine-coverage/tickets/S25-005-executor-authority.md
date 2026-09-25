# S25-005 — executor 선언과 worker authority

- 상태: PLANNED. 우선: **P0 H30**, 나머지 P1. 선행: [S25-001](S25-001-contract-boundaries.md). 소유: host/executor owner; public port 변경은 contract/API owner.
- 주 담당 시나리오: B25, H15, H27, H30, D16.

## 목적

`Builder::build()`가 검증한 executor 선언을 runtime의 유일한 dispatch 선언으로 만들고 실제 worker closure 수명과 Governor lease를 일치시킨다. 현재 `builder.rs`는 `capabilities()`를 검증하지만 `runtime.rs::plan()`은 매 제출마다 domain을 재조회하고 `expect`한다. `Debug`와 공개 `executor_capabilities()`도 재조회한다. 상태가 변하는 adapter의 `None` domain은 panic 후보이고, worker 수/submit 선언 변경은 재검증되지 않는다. `CpuExecutor::spawn`은 `()` 반환 port라 거절 ack가 없으며, closure를 받은 뒤 panic해도 재제출할 수 없다.

## 변경 파일

| 구분 | 경로 | 변경 목적 |
|---|---|---|
| 기존 구현 | `crates/taskmesh/src/builder.rs` | 검증 descriptor를 `TokioRuntime` 생성자에 한 번 전달. resolved topology는 Governor 권위로 유지. |
| 기존 구현 | `crates/taskmesh/src/runtime.rs` | descriptor 보관; `plan()`, `Debug`, `executor_capabilities()`가 같은 사본 사용. `ExecutionLease` accepted/running/drop과 Tokio-context preflight 경계 점검. |
| 기존 fixture | `crates/taskmesh/tests/hardening_executor_protocol.rs`, `hardening_executor_authority.rs`, `hardening_deadline_custody.rs`, `runtime_cpu_executor.rs` | mutable descriptor, closure accepted/drop/panic, bounded shared executor 관측. |
| 기존 topology | `crates/taskmesh-contract/src/topology.rs` | D16은 `try_resolved_cpu_workers(available)`의 주입 가능 seam으로 다른 machine의 resolved 차이를 검증. |
| 신규 후보 | `crates/taskmesh/tests/hardening_executor_capability_snapshot.rs` | 기존 fixture를 비대하게 만들 경우 H30 matrix를 별도 격리. |
| 조건부 port/adapter | `crates/taskmesh-contract/src/ports.rs`, `crates/taskmesh/src/executor/tokio_exec.rs`, `crates/taskmesh-rayon/src/lib.rs` | 설치 후 descriptor 불변 계약의 rustdoc; `spawn` ack가 정말 필요할 때만 API owner가 semver 평가 후 변경. |
| 계약 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | 검증 선언의 권위와 공유 pool 한계 명시. accepted ADR 0003은 과거 사실로 유지. |

## 구현 순서

1. H30 반례를 먼저 고정한다. test adapter는 build 첫 호출에 유효한 descriptor를 반환하고 이후 domain `None`/다른 등록 domain, workers 축소, `nonblocking_submit=false`로 각각 변한다. IO/local/blocking/CPU를 교차 제출하며 panic, 실제 closure 시작, permit/queue/pool 원장을 센다.
2. build 단계에서 검증한 `ExecutorCapabilities` 사본을 runtime에 저장한다. `plan()`은 저장된 `physical_domain`만 전달하고 공개 반환 API와 `Debug`도 같은 사본을 보여준다. 새 pool/queue는 만들지 않는다.
3. 선언 고정과 adapter 실제 행위를 분리한다. 바뀐 `spawn` 동작이 선언을 어기면 adapter protocol 위반이다. 위반 관측 fixture를 두되 host가 외부 공유 pool 전체를 통제한다고 주장하지 않는다.
4. S25-001 D-S25-07에 따라 Tokio 밖 default `run_blocking`/CPU poll 지원 범위를 확정한다. 미지원이면 context를 admission 및 worker 생성 **전에** 검사해 typed 오류를 반환한다. Rayon/custom CPU의 자체 context 요구는 별도 결정한다.
5. `spawn` panic/closure 보류/실행/drop의 lease owner를 점검한다. panic 뒤 retry를 금지하고 작업은 최대 한 번 실행한다.

## DoD

- [ ] `S25-005-H30`: post-build domain/worker/submit 선언 변화와 네 진입점 matrix가 panic 없이 끝난다. 공개 descriptor와 preflight domain은 build 당시 값 하나이고, 각 작업의 시작 횟수·admitted/rejected 결과·role/physical charge가 정확하다. adapter의 실제 `spawn` 거짓말은 별도 protocol 위반으로 표시한다.
- [ ] `S25-005-B25`: Tokio 밖 `run_blocking` 및 기본 CPU future의 **실제 poll** 결과가 합의된 typed 오류이며 worker 시작, permit, ticket, queue, pool charge가 0이다. Rayon/custom CPU는 control로 분리한다. future 구성만 하는 fixture는 불합격.
- [ ] `S25-005-H15`: (a) `spawn` 전 panic, (b) closure 소유 뒤 panic, (c) closure 보류 뒤 caller drop, (d) closure 실행/drop에서 사용자 job 실행 ≤1. 살아 있는 closure는 lease 보유, drop/종료 뒤 정확히 한 번 반환한다. `spawn`에 없는 거절 ack를 가정하지 않는다.
- [ ] `S25-005-H27`: worker 하나인 실제 bounded 공유 executor에 동일 `Arc`를 설치한 두 runtime이 동시 제출한다. runtime별 charge ≤1, 합산 accepted=2가 가능하지만 실제 running physical worker ≤1임을 barrier와 독립 active/peak counter로 보인다. global admitted 상한 1을 주장하지 않는다.
- [ ] `S25-005-D16`: `CpuMode::Auto` config JSON 왕복은 portable mode를 보존한다. `TopologyConfig::try_resolved_cpu_workers(available)`에 서로 다른 availability를 넣어 capacity 차이를 증명하고, 실제 build의 Governor snapshot limit과 설치 descriptor가 별도 authority임을 확인한다. config JSON을 capacity receipt로 쓰지 않는다.

## 계획된 검증

구현 시 owner-local: `cargo test --locked -p taskmesh --test hardening_executor_protocol`, `cargo test --locked -p taskmesh --test hardening_executor_authority`, `cargo test --locked -p taskmesh --test hardening_deadline_custody`, `cargo test --locked -p taskmesh --test runtime_cpu_executor`; 신규 binary면 해당 `--test` 추가. `cargo test --locked -p taskmesh --features rayon --test hardening_executor_authority`로 feature 경계를 확인한다. 그 뒤 `just test`/CI 선택은 S25-012가 맡는다. 이번 계획 작성에서 테스트는 실행하지 않는다.

## 인계·중단 조건

S25-006에 validated descriptor와 lease 전이, S25-008에 physical-domain authority, S25-012에 Rayon test selection을 넘긴다. port 반환형 변경이나 새 전역 pool이 필요하면 S25-001/API owner에게 semver·topology 설계를 먼저 넘긴다. `capabilities()` 재조회 제거만으로 거짓 adapter의 물리 실행 보장을 얻었다고 닫지 않는다.
