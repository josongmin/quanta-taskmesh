# H16-007 — Fairness reference·bounded cost·credit 정합성

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: F — Fairness owner (실제 assignee 미지정)
- 선행 완료: [H16-006](H16-006-effects-and-bounded-promotion.md)
- 원본 finding: [TM16-012](../../../bugbash/sep-16-general/tickets/TM16-012-drr-unbounded-work-under-lock.md), [TM16-013](../../../bugbash/sep-16-general/tickets/TM16-013-wfq-zero-increment.md), [TM16-014](../../../bugbash/sep-16-general/tickets/TM16-014-wfq-cancelled-service-debt.md), [TM16-037](../../../bugbash/sep-16-general/tickets/TM16-037-drr-idle-credit-accumulation.md)
- 배타적 write lease: `engine`; [적용 순서](EXECUTION.md) 준수

## 목적

supported numeric domain에서 credit·서비스 비율·취소·idle 재진입과 dispatch 작업량을 함께 검증한다.

## 변경 범위

- 기존: [crates/taskmesh-engine/src/features/fairness/scheduler.rs](../../../../crates/taskmesh-engine/src/features/fairness/scheduler.rs)
- 기존: [crates/taskmesh-engine/src/features/fairness/retry_after.rs](../../../../crates/taskmesh-engine/src/features/fairness/retry_after.rs)
- 기존: [crates/taskmesh-engine/tests/fairness_drr_deadline.rs](../../../../crates/taskmesh-engine/tests/fairness_drr_deadline.rs)
- 기존: [crates/taskmesh-engine/tests/fairness_weighted.rs](../../../../crates/taskmesh-engine/tests/fairness_weighted.rs)
- 기존: [crates/taskmesh-bench/tests/fairness_property.rs](../../../../crates/taskmesh-bench/tests/fairness_property.rs)
- 제안 경로: `crates/taskmesh-engine/tests/hardening_fairness_reference.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [ ] 작은 입력용 독립 DRR/WFQ reference와 admission event trace를 먼저 만든다. production helper 복사만으로 oracle를 만들지 않는다.
- [ ] DRR empty busy-period reset을 last-pop/last-abandon hook에 연결한다. nonempty capacity-blocked queue는 credit을 유지한다.
- [ ] DRR empty rounds를 산술 skip하고 cursor partial-round 순서를 보존한다. visits는 active class 수에 대한 명시적 bound로 측정한다.
- [ ] WFQ는 지원 weight 범위/scale/u128 tag/rebase 또는 fractional remainder 방식을 결정한다. max(1) 보정으로 모든 큰 weight를 동일화하지 않는다.
- [ ] head/middle/tail cancellation의 미실행 debt를 제거한다. 재계산이 O(Q)라면 Q 상한·한 transition 작업 budget·continuation을 문서화한다.
- [ ] retry-after zero/normalization을 scheduler domain과 일치시킨다. 기존 heuristic을 실제 service-time prediction이라고 부르지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- DRR ring을 **산술적으로** 순회한다. 각 클래스의 다음 affordable visit을 closed form으로
  계산해 가장 이른 것을 고르고, 건너뛴 round가 주었어야 할 quantum을 일괄 credit한다.
  같은 선택·같은 deficit, `O(classes)`.
- queue가 *drain*되는 전이에서 DRR deficit과 WFQ baseline을 reset한다. capacity-block은
  busy period를 끝내지 않으므로 credit을 잃지 않는다.
- 취소/제거 시 `last_finish_tag`를 남은 queue에서 재계산한다. head/middle/tail 어디서
  제거해도 정확하며 남은 요청의 상대 순서는 보존된다.
- WFQ fixed-point scale을 `2^64`로 올려 `1..=u32::MAX` 전 범위에서 increment가 0이 되지
  않는다. weight `0`은 construction에서 거절한다.

Regression: `crates/taskmesh-engine/tests/hardening_fairness_reference.rs` (8),
그중 DRR 순서는 파일 안의 **독립 reference 구현**(교과서적 visit-by-visit)과 정확히
비교한다 — production helper를 복사하지 않았다.

Mutation: 3건 모두 의도한 assertion으로 kill됨 (WFQ scale 되돌리기, busy-period reset
제거, cancellation rebase 제거). 되돌림 해제 후 control PASS.

## 검증 / 완료 조건

- [x] `H16-007-A01` 동일 weight ratio scaling이 같은 순서를 유지
- [x] `H16-007-A02` warmup 0/1/20 후 backlog가 idle credit만큼 peer를 밀지 않음
- [x] `H16-007-A03` cost>quantum/MAX에서 cost/quantum 비례 루프 없음 — count oracle
      (`Governor::drr_ring_visits`, `drr_examines_each_runnable_class_once_per_selection`) + u32::MAX는 5초
      bounded thread; mutation `drr-walks-the-ring-visit-by-visit`
- [x] `H16-007-A04` cancel burst 후 실제 service와 charge 정합
- [x] `H16-007-A05` best-effort tier와 class 내부 FIFO 보존 — 감사(A1-P1-1) 후 promotion budget 경계와
      continuation gap의 newcomer까지 포함 (`drr_order_is_exact_across_the_promotion_budget_boundary`,
      `a_newcomer_queues_behind_a_runnable_head_of_its_own_class`); 동어반복이던 extreme-weight/cancel-middle
      test는 정확한 순서를 단언한다
- [x] `H16-007-A06` 같은 ordered trace 결과가 deterministic

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test fairness_drr_deadline --test fairness_weighted --test fairness_best_effort --test retry_after_property
cargo test -p taskmesh-bench --test fairness_property
```

## 호환성 / 실패 모드

- 전역 budget 때문에 runnable class가 바뀌는 경우까지 oracle에 포함한다.
- FIFO로 fallback하여 테스트를 통과시키는 것은 fairness 기능 보존이 아니다.

## 인계 / 완료 증거

- [ ] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다.
- [ ] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다.
- [ ] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다.

