# 0002. Deficit round-robin은 priority가 아니라 proportional ring이어야 한다

- 상태: Accepted
- 날짜: 2026-06-04
- 결정자: Song Min
- 선행: [0001 — Feature-sliced 헥사고날 아키텍처](0001-hexagonal-feature-sliced-architecture.md)
- 관련 티켓: [T04 — core fairness and retry-after](../plans/jun-4-startup/T04-core-fairness-and-retry-after.md)

## Context

`FairnessPolicy::DeficitRoundRobin { quantum }`의 의도(T04)는 명시적이다.

- T04 티켓: *"DRR rotates across classes deterministically"*, 자료구조
  `BTreeMap<TaskClass, DeficitState>` + `VecDeque<TaskClass>` (active class **ring**).
- 시작 세트 README: *"deficit counter + **round-robin active class ring**"*.

즉 quantum 비율에 비례한 서비스(bounded service difference)를 주는 **classic DRR**이
목표다. quantum이 우선순위로 작동해서는 안 된다.

### 근본 원인 (RCA)

기존 `drr_select`는 ring 없이 다음으로 구현되어 있었다.

1. 매 dispatch마다 **모든** runnable 클래스에 quantum을 credit,
2. **deficit가 가장 큰** 클래스를 서비스, head cost만 debit.

이 구조에서 `cost = 1`, `quantum_a = 1`, `quantum_b = 3`일 때:

- `b`는 step당 `+3 credit − 1 debit = 순 +2`로 deficit가 쌓이고,
- `a`는 `+1`만 쌓인다.

따라서 `b`의 deficit가 `a`를 영원히 추월한 채로 유지되어, `b`의 큐가 완전히 빌
때까지 `a`는 한 번도 디스패치되지 않는다. 결과는 **quantum-priority**이지
proportional interleave가 아니다.

이 결함이 그동안 드러나지 않은 이유: 회귀 테스트
`drr_rotates_across_classes_deterministically`가 **모든 quantum을 1(균등)** 으로
두었기 때문이다. 균등 quantum에서는 deficit가 균형을 유지해 `[a,b,c,a,b,c]` 회전이
나오고, 이는 잘못된 구현과 올바른 구현 **양쪽 모두** 통과시킨다. 결함은 quantum이
서로 다를 때에만 발현하는데 그 경로를 단언하는 테스트가 없었다.

(발견 경위: inferno 적대적 스위트에서 `quantum 1:3`을 단언하려다 실제 디스패치가
`b×12 → a×12`로 나오는 것을 관측.)

## Decision

`drr_select`를 **active-class ring을 가진 classic DRR**로 교체한다. 엔진은 promotion
당 한 건을 반환하므로, ring 상태(`GovernedState.drr_cursor` + 클래스별 `deficit`)를
호출 간에 영속한다.

1. **continue**: cursor가 가리키는 클래스의 deficit가 head cost를 덮는 동안 그
   클래스를 계속 서비스한다 → quantum-N 클래스가 연속 ~N회 디스패치된다.
2. **advance**: deficit가 소진되거나(또는 클래스가 비면) cursor를 **클래스 정렬
   순서로 다음 runnable 클래스**(wrap-around)로 전진시키며, 도착할 때 한 번 quantum을
   credit한다.

`pool`은 클래스 `BTreeMap` 순회로 만들어져 정렬되어 있으므로 "cursor 다음 클래스
(wrap)"가 곧 다음 ring slot이며, 전체 결정이 결정적이다. cursor는 budget이 풀릴 때만
호출되는 단일 슬롯 promotion 경로에서 영속된다.

### 성질

- **proportional**: quantum 1:3, 1:3 backlog → `[a,b,b,b]` 반복, 두 클래스가 동시에
  소진된다.
- **균등 quantum → plain round-robin**: 기존 `[a,b,c,a,b,c]` 동작 보존.
- **bounded & no starvation**: 모든 backlogged 클래스는 매 lap마다 서비스된다.
- **deterministic**: HashMap 순서 의존 없음, BTreeMap 정렬 ring.

## Consequences

- `GovernedState`에 `drr_cursor: Option<TaskClass>` 추가(WFQ `virtual_time`와 동급의
  per-governor 스케줄러 상태).
- 회귀 가드 추가: `fairness_drr_deadline::drr_serves_proportional_to_quantum_not_priority`
  (불균등 quantum 비례성). 기존 균등-quantum 회전 테스트는 그대로 통과.
- WFQ/EDF/FIFO/best-effort 경로는 영향 없음. retry-after의 quantum 항도 무관(별개 관심사).
- public API/타입 변화 없음(`drr_cursor`는 엔진 내부 상태).

## Lesson

행동 테스트는 **결함이 발현하는 파라미터 영역**을 단언해야 한다. 균등 quantum DRR
테스트는 회전을 "검증"하는 것처럼 보였지만 비례성과 우선순위를 구분하지 못했다.
discipline 테스트는 항상 비대칭(서로 다른 weight/quantum/deadline) 케이스를 포함한다.
