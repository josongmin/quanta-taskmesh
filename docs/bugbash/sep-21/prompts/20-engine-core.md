# Prompt — E01/E02/E03/E04 engine-core owner

Assigned tickets: `SEP21-E01`, `SEP21-E02`, `SEP21-E03`, `SEP21-E04`.

당신은 SEP-21 `engine-core`의 단일 writer다. E01→E02→E03→E04를 같은 integration diff에서
구현한다. capability authority, effect custody, memory sequence, pending resolver를 별도 패치로
누적하지 말고 하나의 engine state machine으로 수렴시킨다.

## 먼저 읽을 것

- `AGENTS.md`
- E01, E02, E03, E04 ticket 원문 전체
- findings TM21-001,002,004,008,009,016,023
- engine `state.rs`, `governor.rs`, admission/composite/fairness/memory/inventory modules
- 관련 deterministic tests와 snapshot/public engine API

## write scope

- `crates/taskmesh-engine/**`
- engine-owned tests/fixtures
- E01/E02/E03/E04 ticket 상태와 closure evidence

수정 금지:

- `crates/taskmesh-contract/**` — C01 contract를 소비만 한다.
- `crates/taskmesh/**`, `crates/taskmesh-rayon/**`
- shared docs
- proof/receipt/CI files

## 구현 순서

### E01

- `Ungated`와 registry가 발급한 `Registered(CapabilityId)`를 타입으로 분리한다.
- admission-time dynamic intern과 missing-limit→0/unlimited를 제거한다.
- configured bounded/explicit-unbounded policy와 snapshot이 동일 registry record를 사용하게 한다.
- unknown/empty/typo는 state 변화 없이 typed reject한다.

### E02

- mutex 안에서는 pure state delta와 `TransitionEffects`만 만든다.
- wake와 final destructor를 각각 panic 격리하고 batch 전체 cleanup을 보장한다.
- claim custody 전달 실패는 permit/ticket/capability/resource를 exactly once unwind하고 terminal
  outcome을 발행한다.
- unbounded retirement queue, panic swallow, `mem::forget`를 사용하지 않는다.

### E03

- wrapping epoch를 checked monotonic measurement domain으로 바꾼다.
- exhaustion validation을 aggregate mutation보다 먼저 수행한다.
- explicit/implicit update가 같은 validator와 terminal outcome을 사용하게 한다.

### E04

`C01_CONTRACT_READY`를 받은 뒤 시작한다.

- `CapacityAssessment`/`BlockerSet`/`CycleWitness`를 단일 owner로 만든다.
- full blocker set을 계산하고 immediate-parent lineage가 보유한 irreversible dependency를
  admission과 promotion 양쪽에서 판정한다.
- bounded `CapabilityRequirementSet` 전체를 atomic charge/release한다.
- fairness scheduler는 eligibility를 재구현하지 않고 eligible ordering만 담당한다.
- head-only HOL을 제거하되 scan bound와 deterministic reduce policy를 명시한다.
- queued child가 promotion 시 새 cycle이면 terminalize+wake한다.
- current pending status를 host가 typed API로 읽게 한다.

## interface rule

C01 contract가 부족하면 engine에 string field나 dual representation을 만들지 않는다. 필요한
symbol과 invariant를 `CONTRACT_INTERFACE_REQUEST`로 C01 owner에게 보낸다. host projection은 H02/H03
owner에게 요청하고 engine에서 Tokio 타입을 받지 않는다.

## 검증

각 ticket의 명시된 test와 intentional negative fixture를 lane target에서 실행한다.

```sh
CARGO_TARGET_DIR=target/sep21/engine-core cargo test --locked -p taskmesh-engine
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

panic/reentrant/barrier test에 sleep을 oracle로 쓰지 않는다. 완료 snapshot에서 permit, ticket,
queue, class, capability, CPU, memory, root attribution 보존을 검사한다.

## handoff

E01-A01~A05, E02-A01~A05, E03-A01~A04, E04-A01~A08을 개별 증거에 연결한다.

```text
signal: E04_RESOLVER_READY
engine API: <exact symbols and invariants>
changed paths: <list>
state-machine proof: <tests/negative fixtures/counts>
capacity conservation: <snapshot result>
panic/cycle/fairness results: <exact variants>
documentation delta for R01: <list>
unresolved: <none or exact blocker>
```

engine PASS는 host behavior 또는 release qualification이 아니다.
