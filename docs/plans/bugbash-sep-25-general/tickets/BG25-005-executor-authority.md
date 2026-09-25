# BG25-005 — executor descriptor와 worker authority

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P0
- 선행: BG25-001 D7–D8
- 소유: host/executor owner

## 목적

Builder가 검증한 executor descriptor를 runtime의 단일 불변 authority로 동결하고 Tokio context, shared executor, closure custody 경계를 명확히 한다.

## 근거

- `builder.rs`는 descriptor를 검증하지만 `runtime.rs::plan`과 public accessor가 다시 조회한다.
- stateful custom adapter는 domain `None`으로 panic하거나 worker/domain 선언을 바꿀 수 있다.
- default Tokio descriptor는 실제 ambient blocking pool 관측값이 아니라 Taskmesh submission gate 선언이며 `exclusive_pool=false`다.
- current two-runtime test는 순차 호출과 요청별 새 OS thread라 bounded shared pool을 증명하지 않는다.

## 변경 파일

- `crates/taskmesh/src/{builder,runtime}.rs`
- `crates/taskmesh-contract/src/ports.rs` rustdoc/타입은 API 결정 시
- `crates/taskmesh/src/executor/tokio_exec.rs`, `crates/taskmesh-rayon/src/lib.rs`
- tests: `hardening_executor_{protocol,authority}.rs`, `runtime_cpu_executor.rs`

## 작업 계획

1. build 후 domain/workers/nonblocking 선언을 바꾸는 adapter로 H30을 재현한다.
2. 검증 descriptor를 `TokioRuntime`에 저장하고 plan/debug/accessor가 같은 사본을 사용한다.
3. Tokio context 부재를 admission 전 검사해 합의된 typed error를 반환한다.
4. 실제 bounded shared executor와 barrier로 두 runtime의 per-runtime/aggregate 범위를 관측한다.
5. spawn panic/closure hold/drop/execute에서 job 최대 1회와 lease owner를 검증한다.

## DoD

- `BG25-005-B25`: Tokio 밖 default blocking/CPU poll이 typed preflight error, work/permit/ticket/pool 0으로 끝난다.
- `BG25-005-H15`: spawn 전/후 panic, closure hold/drop/execute에서 user job ≤1, 살아 있는 closure만 lease를 보유한다.
- `BG25-005-H27`: worker 1 공유 executor에서 runtime별 bound와 합산 physical behavior를 독립 peak counter로 관측한다.
- `BG25-005-H30`: descriptor mutation 뒤 네 run path가 panic하지 않고 build-time descriptor 하나만 authority로 쓴다.
- `BG25-005-D16`: Auto config의 portable declaration과 machine-resolved snapshot/executor facts를 구분한다.

## 검증

- Focused host executor tests; default and Rayon feature controls.
- `just test-rayon` selector 확대는 BG25-012가 담당한다.

## 완료 근거

- H30: build 시 검증한 descriptor를 runtime이 동결하며 네 run path, debug, accessor가 adapter를 재조회하지 않는다.
- B25: default blocking/CPU와 timer/local prerequisite가 admission 전에 typed error로 종료되고 governor state가 변하지 않는다. 명시적으로 설치한 built-in Tokio CPU adapter도 동결 descriptor의 `requires_tokio_context`를 통해 같은 검사를 받는다.
- H15: submit panic, accepted closure hold/drop/execute, caller drop에서 user closure 최대 1회와 lease custody를 검증한다.
- H27: worker 1 shared executor의 runtime별 gate와 aggregate peak 1을 barrier 기반 독립 counter로 검증한다.
- D16: portable `Auto` 선언과 build-time resolved executor snapshot을 문서와 accessor에서 구분한다.
- Default와 Rayon focused control, `just dev`, exact-head macOS CI profile을 BG25-012 evidence rail에서 재실행한다.

## 인계 및 중단 조건

- 새 executor별 pool을 추가해 숫자를 맞추지 않는다.
- `CpuExecutor::spawn` 반환형 변경이 필요하면 API/semver owner에게 되돌린다.
