# SEP21-H03 — Installed executor and physical-domain authority

- 상태: PLANNED
- 우선순위: P0
- 포함 finding: TM21-003, TM21-015, TM21-020
- 선행: E01, E04, H01
- write lane: `host`

## 목적

executor 설치 시점에 physical concurrency, nonblocking submission, pool ownership, construction
failure를 함께 검증한다. semantic role quota와 실제 worker domain을 별도 무제한 authority로
두지 않는다.

## RCA

- `ExecutorCapabilities`는 설명 metadata이고 inline `spawn`도 합법이다.
- blocking/maintenance/default CPU fallback은 Tokio shared blocking pool을 쓰지만 논리 role별
  capability만 있고 aggregate physical bound가 없다.
- topology 0은 ungated이며 실제 external pool worker 수와 연결되지 않는다.
- Rayon 문서화 constructor는 이미 있는 fallible path 대신 panic한다.

## 확정 근거

- default topology의 blocking-family 0=unlimited:
  `crates/taskmesh-contract/src/topology.rs:175-184,263-273`.
- default CPU/blocking path가 Tokio shared blocking pool 사용:
  `crates/taskmesh/src/executor/tokio_exec.rs:8-26`,
  `crates/taskmesh/src/runtime.rs:419-442`.
- legacy/blocking submit을 허용하는 port와 synchronous callsite:
  `crates/taskmesh-contract/src/ports.rs:82-94`,
  `crates/taskmesh/src/runtime.rs:719-767`.
- panicking Rayon constructors: `crates/taskmesh-rayon/src/lib.rs:26-60`.

## 목표 구조와 불변식

- worker를 만드는 모든 dispatch는 finite nonzero `PhysicalDomainId` 하나에 귀속된다.
- shared Tokio blocking family는 aggregate domain bound 하나를 atomic charge한다.
- role quota가 필요하면 physical domain과 같은 engine transition에서 함께 charge/release한다.
- installed `CpuExecutor`는 nonblocking submit을 보장해야 하며 false/legacy는 Builder가 거부한다.
- executor/pool/topology build failure는 typed error다.
- class/engine-specific pool, host semaphore, hidden queue를 추가하지 않는다.

## 작업 플랜

1. `crates/taskmesh-contract/src/ports.rs`, `topology.rs`, `config.rs`, `snapshot.rs`
   - installable executor descriptor와 finite Auto/Fixed physical domain contract를 정의한다.
   - build 오류는 기존 config/topology error domain 또는 host builder error로 표현하고 C01의
     plan-validation 파일과 `verdict.rs`를 수정하지 않는다.
2. `crates/taskmesh/src/builder.rs`
   - detected parallelism을 한 번 resolve하고 executor/domain/registry를 함께 validate한다.
3. `crates/taskmesh/src/execution_plan.rs`
   - 실제 dispatch→physical domain을 H01 plan에 freeze한다.
   - E01 registered handle을 build 때 resolve하고 promotion 때 raw name으로 재해석하지 않는다.
4. E01의 registered capability와 E04의 `CapabilityRequirementSet`을 사용해 physical+role
   capacity를 표현한다. H03은 engine 파일을 직접 수정하거나 별도 counter를 추가하지 않는다.
5. `crates/taskmesh/src/runtime.rs`, `executor/tokio_exec.rs`
   - accepted adapter submit은 nonblocking이라는 contract만 소비한다. trampoline queue 금지.
6. `crates/taskmesh-rayon/src/lib.rs`
   - docs/examples를 `try_from_topology`로 전환하고 panicking constructor를 deprecate/remove한다.
7. runtime inventory는 host-owned machine-readable source로 갱신하고, README/external
   interface/library spec/ADR/CHANGELOG delta는 closure evidence로 R01에 넘긴다.
8. `crates/taskmesh/src/lib.rs`와 default/rayon/MSRV consumer fixture에서 C01 contract와 H01
   dispatch API의 facade re-export를 한 번만 통합한다.

## 테스트 플랜

- 신규 `crates/taskmesh/tests/hardening_executor_authority.rs`
  - blocking+maintenance+CPU fallback barrier 동시 실행의 max active가 shared bound 이하.
  - dedicated stack와 owned Rayon domain 분리.
  - config/snapshot declared/actual domain 일치.
- `hardening_executor_protocol.rs`
  - legacy/inline/false submit, unknown workers, fewer workers mismatch typed build reject.
- `hardening_deadline_custody.rs`
  - rejected inline adapter와 Tokio heartbeat 지속.
- `runtime_cpu_executor.rs`, `crates/taskmesh-rayon/tests/rayon_smoke.rs`
  - fallible topology/pool construction과 forced failure seam.
- default/rayon/MSRV/doc/mutation matrix.

## DoD

- `SEP21-H03-A01`: default runtime의 모든 worker-creating domain limit이 finite하고 observable하다.
- `SEP21-H03-A02`: mixed blocking-family actual concurrency가 shared bound를 넘지 않는다.
- `SEP21-H03-A03`: declaration/actual mismatch와 blocking submit adapter가 build-time reject된다.
- `SEP21-H03-A04`: accepted adapter가 Tokio worker를 submit 동안 block하지 않는다.
- `SEP21-H03-A05`: Rayon resource failure가 facade에서 typed error로 반환된다.
- `SEP21-H03-A06`: release/drain 뒤 domain/class/resource accounting이 모두 0이다.

## 호환성과 rollout

- 기존 0=ungated field 의미를 몰래 바꾸지 않는다. 새 versioned Auto/Fixed domain field와
  migration을 제공한다.
- inline/legacy custom executor는 behavioral break다. R01 CHANGELOG input과 consumer fixture가
  필요하다.
- `new/from_topology` deprecation window와 `try_*` before/after를 제공한다.

## 금지되는 임시방편

- run path별 semaphore, class별 pool, `spawn_blocking` trampoline.
- arbitrary nonzero default만 추가하고 shared aggregate를 방치.
- capability flag를 읽지 않고 신뢰하거나 constructor panic을 catch해 typed인 척하기.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh --test hardening_executor_authority --test hardening_executor_protocol --test runtime_cpu_executor
cargo test --locked -p taskmesh --features rayon
cargo test --locked -p taskmesh-rayon
just consumer-msrv
```
