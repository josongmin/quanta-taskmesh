# Jun-4 Startup Ticket Set

## Goal

이 세트의 목표는 `/Users/songmin/Documents/code-new/quanta-taskmesh`를 `compileable skeleton`에서 `usable governed runtime library`로 끌어올리는 것이다.

범위:

1. `taskmesh` 외부 레포 자체 구현만 다룬다.
2. Semantica downstream wiring은 다루지 않는다.
3. public naming은 현재 skeleton 기준을 유지한다.
4. `taskmesh-tower`는 제외한다.
5. `taskmesh-rayon`은 포함한다.

현재 시작점:

1. `taskmesh-contract`
   - public type skeleton은 존재한다.
   - placeholder/validation/test coverage가 부족하다.
2. `taskmesh-core`
   - inflight accounting과 basic reject만 있다.
   - bounded queue, fairness, memory semantics, leak sweep, composite attribution이 비어 있다.
3. `taskmesh` facade (`crates/taskmesh-tokio`)
   - `run_io/run_blocking/run_cpu/run_local` skeleton은 있다.
   - timeout/cancel adapter, executor abstraction, inventory guard, docs/test proof가 없다.

## Dependency Order

`T01 -> T02 -> T03 -> T04 -> T05 -> T06 -> T07 -> T08 -> T09 -> T10`

설명:

1. `T01`이 public contract를 고정한다.
2. `T02`가 config/validation을 닫는다.
3. `T03~T06`이 core semantics를 쌓는다.
4. `T07~T09`가 runtime/executor를 붙인다.
5. `T10`이 proof/doc/release gate를 닫는다.

## Ticket Graph

1. `T01-contract-freeze`
   - downstream: `T02`, `T05`, `T06`, `T10`
2. `T02-config-builder-and-validation`
   - downstream: `T03`, `T05`, `T07`, `T10`
3. `T03-core-admission-queue-permit`
   - downstream: `T04`, `T06`, `T07`
4. `T04-core-fairness-and-retry-after`
   - downstream: `T07`, `T10`
5. `T05-memory-governance-and-leak-sweep`
   - downstream: `T07`, `T10`
6. `T06-composite-task-stage-and-reduce`
   - downstream: `T07`, `T09`
7. `T07-tokio-runtime-adapter`
   - downstream: `T08`, `T09`, `T10`
8. `T08-substrate-inventory-and-local-runtime`
   - downstream: `T10`
9. `T09-rayon-executor-adapter`
   - downstream: `T10`
10. `T10-ci-proof-docs-release`
   - final closeout

## Global No-Go Rules

1. public type rename 금지
   - `TaskSpec`, `TaskClass`, `TaskStage`, `SubstrateHint`, `AdmissionVerdict`, `GovernorError`, `RunError<E>`, `Runtime`
2. engine-specific pool 추가 금지
3. unknown class default-admit 금지
4. competing path unbounded queue 금지
5. raw `tokio::spawn`을 runtime facade 우회 수단으로 도입 금지
6. `spawn_blocking` hardcode를 phase-2 final shape라고 주장 금지
7. deterministic reduce policy 없는 parallel stage closeout 금지
8. telemetry-only를 memory governance라고 부르지 않음
9. `taskmesh` public API에 product-local taxonomy 추가 금지
10. downstream integration code를 이 startup 세트에 끼워 넣지 않음

## Closeout Gate

완료 기준:

1. `cargo check`
2. targeted unit tests
3. docs example compile
4. placeholder public type zero
5. public naming drift zero
6. `taskmesh-rayon` 포함 workspace green
7. README / spec / external interface가 실제 코드와 일치

필수 proof 시나리오:

1. unknown class reject
2. max inflight saturation
3. queue depth saturation
4. retry-after fixed/adaptive
5. memory overcommit reject/queue/degrade
6. child permit root attribution
7. recursive admission rejection
8. deterministic reduce policy enforcement
9. `run_local` non-`Send` path
10. snapshot contains class/resource/substrate state
11. config validation failure on impossible budgets
12. docs example compile

## Paths and Naming Notes

1. facade package name은 `taskmesh`다.
2. facade source path는 `crates/taskmesh-tokio/src/lib.rs`다.
3. optional CPU executor adapter는 `crates/taskmesh-rayon`에 둔다.
4. owner-local tests 우선 원칙으로 각 crate 아래 `tests/` 디렉터리를 기본 대상으로 삼는다.

## Concrete Technical Defaults

사용 라이브러리:

1. `serde`
   - contract wire types
2. `serde_json`
   - roundtrip tests, allowlist fixture parsing
3. `parking_lot`
   - core state mutex
4. `tokio`
   - async host, `spawn_blocking`, `LocalSet`, `timeout`, `oneshot`
5. `tokio-util`
   - `CancellationToken`
6. `rayon`
   - shared CPU executor adapter

기본 자료구조:

1. `BTreeMap`
   - class keyed state, deterministic ordering
2. `VecDeque`
   - per-class FIFO queue
3. `BTreeSet`
   - deterministic uniqueness / active composite tracking
4. `AtomicU64`
   - permit id / sequence number

기본 알고리즘:

1. queue discipline
   - per-class `VecDeque<PendingRequest>`
2. weighted fairness
   - virtual finish time 기반 `WeightedFairQueue`
3. DRR
   - deficit counter + round-robin active class ring
4. deadline-aware
   - `(deadline_ms, seq_no)` 기준 min-order dispatch
5. best-effort scavenger
   - non-best-effort runnable class가 없을 때만 dispatch

금지:

1. `HashMap` iteration order 의존
2. engine-specific pool
3. core crate 안의 Tokio primitive 의존
4. runtime facade 우회 direct spawn
