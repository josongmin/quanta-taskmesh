# S25-009 — queue·scheduler·drain 경합

- 상태: PLANNED; 우선: P1; 선행: [S25-001](S25-001-contract-boundaries.md), [S25-008](S25-008-admission-capacity.md); 소유: engine/fairness owner, drain host owner.
- 주 담당 시나리오: B23, B27, H04, H06, H13, H14, H16, H25, D19.
- 근거: `governor.rs`의 ticket/permit transition, `pending_view`, `runtime/drain.rs`, `hardening_effect_retirement.rs`, `differential_model.rs`. 기존 differential model은 memory/fairness/child/waker/reap/promotion-budget을 제외한다.

## 목적

작은 deterministic state history를 먼저 만들고 각 transition의 linearization point, terminal reason, permit/ticket 독립 ledger를 검증한다. waker/settlement callback은 mutex 밖에서 호출하며 panic 후 보상·나머지 effect 처리 범위를 정의한다. 이후 production `sync` seam을 통과하는 제한된 모델만 Loom/Shuttle에 넣는다. toy replay fixture를 Governor proof로 승격하지 않는다. `docs/taskmesh-library-spec.md`의 `pending_block_reason` intake 고정 설명은 현재 `pending_view`의 재평가 경로와 대조해 H25 결과에 맞춰 갱신한다.

## 변경 파일

| 구분 | 파일 | 조치 |
|---|---|---|
| 신규 engine fixture | crates/taskmesh-engine/tests/hardening_queue_history.rs | 1~2 ticket의 release/promotion/claim/abandon/timeout/sweep 이력, 독립 permit·ticket 원장, 재현 seed. |
| 신규 host fixture | crates/taskmesh/tests/hardening_drain_multiwait.rs | 동일 runtime의 drain caller 두 명과 close/snapshot/wait 경합, direct Governor settlement. |
| 기존 회귀 | crates/taskmesh-engine/tests/hardening_effect_retirement.rs, hardening_fairness_reference.rs, pending_resolver.rs, differential_model.rs; crates/taskmesh/tests/hardening_drain.rs | 단일 경합/독립 scheduler oracle 범위를 보존하고 신규 history와 비교. |
| 조건부 source | crates/taskmesh-engine/src/engine/governor.rs, engine/state.rs, features/admission/mod.rs, features/admission/pending.rs, features/fairness/scheduler.rs, sync.rs; crates/taskmesh/src/runtime/drain.rs | 실제 반례가 확인된 transition, effect retirement, scheduler 또는 wake 경로만 수정. |
| 조건부 model/docs | crates/taskmesh-engine/tests/loom_governance.rs, shuttle_governance.rs; tools/modelcheck/run.py, producer-manifest.json; docs/taskmesh-library-spec.md | 결정적 history가 생긴 뒤 production-linked 작은 schedule 추가. 문서의 pending blocker 시점은 관측과 맞춤. Gate inventory는 S25-012 소유. |

## 구현 순서

1. S25-008의 capacity/blocker 계약을 입력으로, queued·promoted-unclaimed·claimed permit·terminal ticket을 구분하는 작은 직렬 event model을 만든다. production State를 복제하지 않는다.
2. release↔claim/abandon/timeout/sweep를 2-thread barrier로 교차하고 각 history가 가능한 직렬 순서 하나와 일치하는지 검사한다. timeout 자체를 liveness 증거로 사용하지 않는다.
3. WFQ/DRR의 weight/cost, 취소, queue drain, cross-pool follower를 기존 reference scheduler와 비교한다. blocked head의 오버테이크는 해당 pool이 다를 때만 허용한다.
4. waker drop/wake/panic을 promotion continuation과 결합해 callback의 lock 밖 실행, 첫 panic 전파, 나머지 effects와 원장 보존을 따로 판정한다.
5. 결정적 반례만 실제 Governor/sync seam의 bounded Loom/Shuttle schedule에 옮긴다. toy replay는 Governor proof로 쓰지 않는다.

## DoD

- [ ] `S25-009-B23` permit 없는 queued ticket의 clock 전진/reap 반복 후 유지와 caller abandon/timeout 또는 capacity release만 terminal/promotion하는 주체를 판정한다.
- [ ] `S25-009-B27` malformed preflight의 final-reference waker destructor panic에서 permit/ticket/queue side effect 0; panic 전파는 transition-effect 보상 계약과 분리한다.
- [ ] `S25-009-H04` release↔promotion↔claim/abandon/timeout/sweep 경합에서 정확히 한 owner와 Ready/Terminal/Invalid 결과를 barrier history로 판정한다.
- [ ] `S25-009-H06` WFQ/DRR weight/cost와 취소/cross-pool follower의 선택 순서를 독립 scheduler oracle과 비교한다.
- [ ] `S25-009-H13` close↔snapshot↔wait 경계와 두 drain caller가 direct release/abandon/reap settlement 후 모두 깨어나는지 확인한다.
- [ ] `S25-009-H14` waker/settlement 재진입·panic과 promotion continuation을 결합해 lock 밖 실행, 보존 원장, 첫 panic 전파를 확인한다.
- [ ] `S25-009-H16` 실제 Governor 두 스레드의 짧은 history를 선형화 가능한 독립 모델과 비교하고 실패 seed/schedule을 replay한다. modelcheck가 관측하지 못하는 외부 동작을 명시한다.
- [ ] `S25-009-H25` class/pool/CPU/memory blocker를 하나씩 제거해 `pending_assessment`·`pending_block_reason`·scheduler 실제 선택을 구분한다.
- [ ] `S25-009-D19` read-only `pending_view`는 committed watermark를 쓰고 전이 뒤 monotonic 갱신되는지 manual clock으로 판정한다.

각 항목의 추가 판정:

- B23: permit 없는 queued ticket는 clock만 전진하고 reap을 반복해도 그대로 남는다. abandon/timeout 또는 실제 capacity 반환만 terminal/promotion시킨다.
- B27: malformed preflight 중 final-reference waker destructor가 panic해도 permit/ticket/queue/held의 커밋 변화가 0이다. panic 전달과 transition effect는 별개로 기록한다.
- H04/H16: 각 짧은 이력은 terminal owner가 한 명이며 유령 permit·이중 release가 없다. 최소 실패 trace와 fixed seed replay가 남고 modelcheck의 제외 범위도 기록된다.
- H06/H25: scheduler의 실제 선택 순서, pending_assessment의 전체 현재 blocker, pending_block_reason의 primary를 서로 구분한다. 취소된 class credit/tag와 cross-pool follower를 독립 oracle로 판정한다.
- H13: 두 drain caller가 마지막 direct release/abandon/reap settlement에 모두 깨고 동일 idle snapshot으로 성공한다. timeout 시 NotDrained는 실제 outstanding과 일치한다.
- H14: callback은 engine mutex 밖에서 재진입하고 첫 panic 뒤에도 문서화된 나머지 effect/claim compensation 범위를 지킨다.
- D19: ManualClock만 이동한 read-only pending_view는 queue_wait_ms를 늘리지 않고, commit transition 뒤 watermark 기준으로 단조 증가한다. query 자체는 상태를 변경하지 않는다.

## 계획된 검증

- Owner-local: cargo test --locked -p taskmesh-engine --test hardening_queue_history; cargo test --locked -p taskmesh --test hardening_drain_multiwait. 기존 effect retirement·fairness reference·pending resolver fixture도 영향에 따라 실행한다.
- 통합: S25-012가 같은 clean HEAD에서 just test-core/just test를 적용한다. production-linked modelcheck·TSan은 별도 nightly rail이며 이 계획 작성 중 실행하지 않는다.

## 인계·중단 조건

- 인계: 계약표, linearization history, replay seed, callback panic 범위, permit/ticket 독립 ledger, 문서 변경점. S25-010은 확정된 release/promotion 순서를 소비한다.
- 재현 불가능한 race를 추측으로 source 수정하지 않는다. callback/public status semantics가 달라져야 하면 S25-001에서 계약과 migration을 먼저 결정한다.
