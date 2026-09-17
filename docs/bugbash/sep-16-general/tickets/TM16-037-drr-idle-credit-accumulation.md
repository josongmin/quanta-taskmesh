# TM16-037 — DRR drained queue의 잔여 credit이 누적되어 이후 burst의 service bound를 깨뜨린다

- Severity: P2
- Status: OPEN / deterministic public-engine reproduction
- Lane: F — fairness; empty-queue transition은 E와 interface 조정
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-engine/src/features/fairness/scheduler.rs:109-110`: empty queue는 candidate에서 제외된다.
- 같은 파일 `:160-168,181-195`: 기존 deficit가 있으면 계속 dispatch하며 다음 ring visit에 quantum을 더한다. idle/drain 시 deficit reset은 없다.
- `crates/taskmesh-engine/src/engine/governor.rs:122-144,167-193`: abandon/pop_front로 마지막 pending request가 제거되어도 residual credit은 남는다.
- [ADR 0002](../../../adr/0002-drr-proportional-fairness.md): classic DRR, proportional service와 bounded service difference가 명시된 계약이다.
- [Retained observation](evidence/repro/src/lib.rs): `drr_empty_queue_retains_credit_across_busy_periods`. validated `Governor::new`와 public admit/release/claim만 사용한다.

## Trigger / 관찰

global CPU=1, class별 cost=1, quantum A=2/B=1, bounded queue=64로 구성한다.

1. B permit을 유지한 채 A/B 각각 1건을 queue한다.
2. B holder를 반환하여 A를 실행한다. A queue는 empty지만 deficit=1이 남는다.
3. A를 반환하여 B를 실행하고 B permit을 다음 cycle holder로 유지한다.
4. 20회 반복 후 A/B 각각 10건을 queue한다. 준비 단계에서도 A queue가 empty임을 assertion으로 확인한다.

- fresh control: `A,A,B,A,A,B,...`.
- 20회 drain 후: `A×10,B×10`. B가 runnable이고 queued 상태여도 A burst 전체가 먼저 실행된다.
- 모든 ticket을 claim/release하며 종료 시 각 class의 queued/inflight=0을 검증한다.

## 원인 / 영향 / 범위

위 cycle은 A가 한 건만 서비스받고 drain되므로 매번 사용하지 않은 quantum 1을 저장한다. 이후 ring visit에서 누적 credit 전체를 소비할 수 있다. 동일한 현재 queue/budget/quantum인데 과거의 intermittent activity 횟수에 비례해 peer service delay가 커진다. 고정 quantum에 따른 burst/service-difference bound를 제공하지 못한다.

이는 단순 scheduler 취향이나 steady backlog의 ratio 문제가 아니다. 무한 starvation을 재현한 것은 아니며, history-dependent burst가 현재 pending backlog를 전부 앞지를 수 있음을 증명했다. cost/quantum loop 작업량인 TM16-012, WFQ cancellation tag인 TM16-014와 별도 원인이다. host ingress arbitration 문제(TM16-001) 없이 engine 단독으로 재현된다.

## 보완 계획

- pending queue가 empty가 되는 전이에 DRR deficit reset을 명시한다. promotion과 마지막 queued request abandon을 모두 포함한다.
- 단순히 runnable candidate에서 빠졌다는 이유로 reset하지 않는다. resource/per-class capacity block은 queue drain과 다르다.
- cursor 보존/advance와 새 busy period의 quantum credit을 reference DRR로 정의한다. 기존 oversized-cost credit 누적은 유지한다.
- F가 scheduling 계약/테스트를 소유하고 E가 필요한 queue mutation hook을 단독 수정한다.

## Acceptance / 회귀 검증

- warmup=0,1,20,large에서 동일한 재진입 backlog가 idle history에 비례하는 service burst를 만들지 않는다.
- unequal/equal quantum, 마지막 pending promotion/abandon, single-class idle 후 peer join을 포함한다.
- nonempty-but-capacity-blocked queue 및 cost>quantum은 정당한 credit을 잃지 않는다.
- corrected behavior를 production fairness tests에 추가한다. retained observation의 green은 결함 재현이며 수정 완료가 아니다.
