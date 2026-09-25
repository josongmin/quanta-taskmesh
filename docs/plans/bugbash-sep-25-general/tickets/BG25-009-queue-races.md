# BG25-009 — queue·fairness·waker·drain histories

- 상태: IMPLEMENTED
- 우선순위: P1
- 선행: BG25-001, BG25-008
- 소유: engine/scheduler owner; drain host owner

## 목적

기존 단일 fairness/effect 테스트를 넘어 release, promotion, claim, abandon, timeout, sweep, cancellation, continuation, drain을 선형화 가능한 bounded histories로 검증한다.

## 근거

- fairness reference와 promotion-budget/waker panic isolated fixtures는 강하다.
- differential model은 memory, DRR/WFQ, waker, child, reap, promotion budget을 제외한다.
- `pending_view`는 현재 assessment를 재계산하며 admission-time blocker를 고정하지 않는다.
- current HEAD modelcheck receipt는 없다.

## 변경 파일

- engine fixture `hardening_queue_history.rs`
- host fixture `hardening_drain.rs`
- 기존 `hardening_{fairness_reference,effect_retirement}.rs`, `pending_resolver.rs`, `differential_model.rs`
- 재현 시 `engine/governor.rs`, fairness/admission state; model changes require producer manifest update

## 작업 계획

1. barriers로 linearization windows를 정하고 input-derived ticket/permit ledger를 유지한다.
2. four-way settlement and two drain waiters를 일반 deterministic test로 먼저 고정한다.
3. WFQ/DRR cancel/cross-pool/promotion-budget histories를 independent scheduler와 비교한다.
4. 작은 재현 상태만 Loom/Shuttle model에 추가한다.

## DoD

- `BG25-009-B23`: queued LeakDetecting ticket은 시간/sweep만으로 사라지지 않고 release 시 promotion 대상이다.
- `BG25-009-B27`: preflight waker-destructor panic은 admission state를 만들지 않는다고 별도 판정한다.
- `BG25-009-H04`: release/promotion/claim/abandon/timeout/sweep에서 ownership exactly once and finite terminal outcome.
- `BG25-009-H06`: WFQ/DRR mixed cost/weight, cancel, cross-pool service matches independent reference.
- `BG25-009-H13`: direct settlement wakes multiple drains without lost wake.
- `BG25-009-H14`: reentrant/panicking callbacks run outside lock and remaining effects/continuation settle.
- `BG25-009-H16`: bounded concurrent history agrees with an independent model and failure schedule is replayable.
- `BG25-009-H25`: blocker-set/current-primary/promotion-pending diagnostics track current state exactly.
- `BG25-009-D19`: queue wait uses committed clock watermark and advances only after transitions.

## 검증

- Default deterministic tests first.
- `just modelcheck` only after producer manifest/scenario set is updated and explicit high-cost run is authorized.

## 인계 및 중단 조건

- Do not treat arbitrary concurrent snapshots as sequential-model steps.
- Engine production changes remain serial with BG25-008 and BG25-010.
