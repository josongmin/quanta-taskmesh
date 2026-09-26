# Prompt — H01/H03/H02 host runtime owner

Assigned tickets: `SEP21-H01`, `SEP21-H03`, `SEP21-H02`.

당신은 SEP-21 `host` lane의 단일 writer다. H01→H03→H02 순서로 preflight, physical executor
authority, acquisition linearization을 하나의 transaction redesign으로 구현한다.

## 먼저 읽을 것

- `AGENTS.md`
- H01, H03, H02 ticket 원문 전체
- findings TM21-003,005,006,007,015,019,020
- host runtime/execution plan/builder/executor code, executor-related contract, Rayon adapter와 tests

## write scope

- `crates/taskmesh/src/runtime.rs`, `execution_plan.rs`, `builder.rs`, executor modules
- executor-related `taskmesh-contract` ports/topology/config/snapshot files
- `crates/taskmesh-rayon/**`
- host/facade/MSRV/default/rayon tests and fixtures
- H01/H03/H02 ticket 상태와 evidence

수정 금지:

- `taskmesh-contract`의 C01 task/validation files
- `crates/taskmesh-engine/**`
- shared docs
- proof/receipt/CI files

## phase A — H01

`C01_CONTRACT_READY` 뒤 시작한다.

- stack/substrate/dispatch validation을 admission 전 `ValidatedDispatchPlan` preflight로 옮긴다.
- 16 GiB upper bound와 `usize::try_from`을 한 owner에서 처리한다.
- sync/background/large-stack path가 같은 typed value를 사용하게 한다.
- invalid plan은 ticket/permit/thread/closure side effect 0으로 reject한다.

H01 완료 후 H03를 임시 shim으로 시작하지 않는다. `E04_RESOLVER_READY`가 아직 없으면 exact H01
evidence를 coordinator에게 보내고 대기한다.

## phase B — H03

`E04_RESOLVER_READY` 뒤 재개한다.

- worker-creating dispatch를 finite nonzero physical domain에 귀속한다.
- shared Tokio blocking family는 aggregate domain bound 하나를 사용한다.
- E01 registered handle과 E04 requirement set으로 physical+role requirement를 표현한다.
- raw capability name을 promotion에서 재해석하지 않는다.
- installed executor의 synchronous/inline submit을 build-time typed reject한다.
- 별도 semaphore, engine-specific pool, trampoline queue를 만들지 않는다.
- Rayon construction failure를 typed error로 반환하고 panicking constructor migration을 제공한다.
- C01/H01 public types의 `taskmesh` facade re-export와 consumer fixtures를 여기서 한 번 통합한다.

## phase C — H02

- cancel token, absolute deadline, relative acquisition budget을 원형대로 보존한다.
- precedence는 cancel→absolute deadline→relative timeout으로 고정하고 equality는 expired다.
- immediate admit와 queued ready가 동일 finalize function을 통과하게 한다.
- handoff 거부는 unstarted permit을 exactly once 반환하고 worker/closure/thread를 만들지 않는다.
- E02 compensated terminal 뒤 `TicketGuard` duplicate abandon/release를 막는다.
- timeout 직전 E04 current blocker set을 읽고 stale intake cause를 폐기한다.

## 검증

ticket에 명시된 default/rayon/MSRV/consumer tests와 deterministic barrier negative를 lane별 target에서
실행한다.

```sh
CARGO_TARGET_DIR=target/sep21/host cargo test --locked -p taskmesh
CARGO_TARGET_DIR=target/sep21/host-rayon cargo test --locked -p taskmesh --features rayon
CARGO_TARGET_DIR=target/sep21/rayon cargo test --locked -p taskmesh-rayon
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

sleep/yield timing test, spawn 후 취소, arbitrary nonzero default, hidden queue를 허용하지 않는다.

## handoff

H01-A01~A05, H03-A01~A06, H02-A01~A06을 각각 증거에 연결한다.

```text
signal: HOST_LANE_CLOSED
dispatch transaction: <exact types/linearization point>
physical domains: <declared/actual mapping>
changed paths: <list>
default/rayon/MSRV results: <commands/counts>
negative fixtures: <results>
documentation delta for R01: <list>
unresolved: <none or exact blocker>
```

host local PASS는 engine proof나 release qualification이 아니다.
