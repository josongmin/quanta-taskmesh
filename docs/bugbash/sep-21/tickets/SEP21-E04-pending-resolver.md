# SEP21-E04 — Unified pending-admission resolver

- 상태: PLANNED
- 우선순위: P0
- 포함 finding: TM21-001, TM21-002, TM21-009, TM21-023
- 선행: C01, E01, E02
- write lane: `engine-core`

## 목적

admission, promotion, nested-wait 판정, timeout diagnostics가 동일한 current-state resolver를
사용하게 한다. root/child 조건문, 첫 blocker, queue front snapshot을 각각 패치하지 않는다.

## RCA

- `capacity_for`는 첫 blocker 하나만 반환한다.
- cycle detector는 root-scoped permit만 보고 immediate parent를 모른다.
- scheduler는 class front 하나만 eligibility 후보로 본다.
- `PendingRequest.blocked_on`은 intake snapshot인데 timeout truth로 사용된다.
- 이 네 projection이 서로 다른 상태를 설명해 deadlock, starvation, 오진이 생긴다.

## 확정 근거

- first-blocker `capacity_for`: `crates/taskmesh-engine/src/engine/state.rs:483-525`.
- 단일 blocker만 cycle detector에 전달:
  `crates/taskmesh-engine/src/features/admission/mod.rs:184-192,260-309`.
- root-scoped permit만 집계하는 detector:
  `crates/taskmesh-engine/src/features/composite/mod.rs:91-145`.
- front-only promotion: `crates/taskmesh-engine/src/features/fairness/scheduler.rs:202-238`.
- promotion의 cycle 재판정 부재: `crates/taskmesh-engine/src/engine/governor.rs:432-480`.
- intake blocker diagnostic과 host timeout projection:
  `crates/taskmesh-engine/src/engine/governor.rs:842-847`,
  `crates/taskmesh/src/runtime.rs:226-235,834-843`.

## 목표 구조와 불변식

- engine-owned resolver 결과:
  `Runnable | ReversiblyBlocked(BlockerSet) | IrreversibleWaitCycle(CycleWitness)`.
- `BlockerSet`은 class/capability/CPU/memory/queued-behind를 동시에 표현한다.
- `CycleWitness`는 validated immediate parent permit과 그 permit이 보유한 실제 capacity를
  증명한다. sibling/stranger capacity는 포함하지 않는다.
- queue selection은 earliest fully-runnable request를 결정적으로 선택하며 동일 capability
  FIFO를 보존한다.
- timeout은 같은 resolver의 current blocker projection을 사용한다.
- scan work는 configured `max_queue_depth` 이하로 bounded다.
- C01 contract validator는 ID allocation과 state lock 전에 한 번 실행된다.
- execution plan은 하나 이상의 registered capability requirement를 담을 수 있고 resolver가
  이를 한 atomic assessment에서 판정한다.

## 작업 플랜

1. 신규 권장 `crates/taskmesh-engine/src/features/admission/pending.rs`
   - `CapacityAssessment`, `BlockerSet`, `CycleWitness`, bounded eligible scan을 구현한다.
2. `crates/taskmesh-engine/src/engine/state.rs`
   - single-blocker `capacity_for`를 assessment owner로 대체한다.
   - active parent operation→permit index와 capability ID를 유지한다.
   - bounded `CapabilityRequirementSet` 전체를 atomic charge/release한다.
3. `crates/taskmesh-engine/src/features/composite/mod.rs`
   - root-scope scan을 제거하고 validated parent witness만 검사한다.
4. `crates/taskmesh-engine/src/features/admission/mod.rs`
   - intake가 resolver 결과만으로 admit/queue/reject를 결정한다.
   - C01 validator를 ID allocation/state mutation 전에 호출하고 duplicate stage를
     `MalformedTask`로 투영한다.
5. `crates/taskmesh-engine/src/features/fairness/scheduler.rs`
   - semantic eligibility를 재구현하지 않고 eligible candidate ordering만 담당한다.
6. `crates/taskmesh-engine/src/engine/governor.rs`
   - promotion 때 assessment를 재계산하고 새 irreversible cycle은 terminalize+wake한다.
7. current typed pending status API를 export한다. host error projection은 H02 owner가 수행하며
   E04는 `runtime.rs`를 수정하지 않는다.

## 테스트 플랜

- 신규 `crates/taskmesh-engine/tests/pending_resolver.rs`
  - descendant parent-held class/pool/CPU/memory cycle 4종.
  - `Inflight+Capability`, `Inflight+CPU`, `Inflight+Memory` blocker 조합.
  - sibling/stranger/reversible blocker no-false-cycle.
  - duplicate same/conflicting stage와 invalid plan의 zero state delta.
- `hardening_fairness_reference.rs`
  - pool A head 뒤 earlier B follower가 later B newcomer보다 먼저 실행.
  - 동일 pool FIFO와 bounded scan.
- `admission_queue.rs`
  - blocker transition별 current pending status. public timeout verdict는 H02에서 검증한다.
- `shuttle_governance.rs`
  - release/cancel/promotion과 wait-cycle terminalization interleaving.
- 각 규칙의 intentional mutant를 curated inventory에 추가한다.

## DoD

- `SEP21-E04-A01`: timeout 없이 모든 declared irreversible cycle이 typed terminal outcome으로 끝난다.
- `SEP21-E04-A02`: reversible sibling/stranger capacity는 false reject하지 않는다.
- `SEP21-E04-A03`: later newcomer가 queued 동일-domain predecessor를 추월하지 않는다.
- `SEP21-E04-A04`: admission과 promotion이 같은 fixture에서 같은 assessment를 낸다.
- `SEP21-E04-A05`: cancel/abandon/terminalization 뒤 fairness debt, recursion index, accounting이 보존된다.
- `SEP21-E04-A06`: current pending status는 마지막 blocker set과 일치하고 intake snapshot을 반환하지 않는다.
- `SEP21-E04-A07`: duplicate/invalid plan은 ID/ticket/permit/scheduler/root state 변화 없이 거절된다.
- `SEP21-E04-A08`: 여러 registered capability requirement가 한 transition에서 atomic charge/release된다.

## 금지되는 임시방편

- `TaskScope::Child` 전부를 parent-held로 계산.
- blocker enum 우선순위만 변경.
- host가 여러 snapshot을 조합해 cause 추측.
- capability별 worker pool 또는 dual-write queue 추가.
- blocked head 무조건 skip 또는 newcomer direct-admit 예외만 제거.
- deadlock test의 timeout을 성공 조건으로 사용.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh-engine --test pending_resolver --test hardening_nested_wait --test hardening_fairness_reference --test admission_queue
cargo test --locked -p taskmesh --test hardening_nested_wait --test runtime_cancel_timeout
just loom
just shuttle
```
