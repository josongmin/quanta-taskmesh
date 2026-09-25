# BG25-009 — queue·fairness·waker·drain histories

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P1
- 선행: BG25-001, BG25-008
- 소유: engine/scheduler owner; drain host owner

## 목적

기존 단일 fairness/effect 테스트를 넘어 release, promotion, claim, abandon, timeout, sweep, cancellation, continuation, drain을 선형화 가능한 bounded histories로 검증한다.

## 근거

- fairness reference와 promotion-budget/waker panic isolated fixtures는 강하다.
- `hardening_queue_history.rs`가 release/promotion/reap/claim/abandon과 clock watermark를 하나의 bounded deterministic history로 검증한다. `hardening_drain.rs`는 두 drain future가 각각 `Pending`까지 poll된 뒤 한 direct release에 함께 깨어나는지 검증한다.
- 기존 differential model은 memory, DRR/WFQ, waker, child, reap, promotion budget까지 포괄하는 전체 model이 아니다. 이 범위의 modelcheck qualification을 주장하지 않는다.
- `pending_view`는 현재 assessment를 재계산하며 admission-time blocker를 고정하지 않는다.
- Deterministic test와 nightly modelcheck는 별도 proof rail이다. current HEAD modelcheck receipt는 없다.

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

- Default deterministic tests는 최종 committed HEAD의 BG25-012 receipt에서 판정한다.
- `just modelcheck` only after producer manifest/scenario set is updated and explicit high-cost run is authorized.

## 최종 의미 감사

- H04는 `release_claim_timeout_cancel_and_sweep_preserve_one_owner`에서 한 Governor의 stale holder와 세 waiter를 만든 뒤 release·claim·timeout abandon·cancel abandon·sweep를 같은 barrier 뒤 병행한다. 32개 bounded attempt마다 release와 sweep의 단일 소유권, claimant의 최종 permit, 취소된 ticket의 무효화, admitted/terminated/queued/CPU 원장을 quiescent cut에서 판정한다. ManualClock finite history와 실제 host timeout race storm은 supporting cases다. 이 fixture는 모든 내부 interleaving의 exhaustive modelcheck를 뜻하지 않는다.
- H06은 DRR heterogeneous cost/quantum reference를 primary로 두고 WFQ non-head service tag, cross-pool service 뒤 cancellation debt, middle cancellation survivor order를 supporting cases로 연결한다.
- H13은 direct release/abandon/reap, finite/unbounded drain wake, release 전 양쪽 `Pending` 등록을 확인한 동시 drain caller, 첫 snapshot과 release 경합 cases를 연결한다. H14는 panicking `PermitWaker`, reentrant wake/drop/release, reentrant·panicking `SettlementWaker`, promotion-budget 초과 backlog의 별도 cases를 연결한다.
- H16은 두 스레드의 동시 admit 및 release/admit/reap 경합을 barrier와 quiescent input ledger로 검사한다. `model_replay_fixture.rs`는 같은 production `sync` seam의 실제 Governor에서 두 admission의 순서가 역전될 때 의도적으로 거짓인 선착순 가정을 실패시키고, Shuttle의 저장된 schedule로 같은 sentinel 실패를 재실행한다. 이 fixture는 실패 기록·재생 파이프라인의 좁은 증거이며 Governor 결함이나 전체 nightly modelcheck PASS가 아니다. 최종 source-bound modelcheck receipt는 별도다.
- H25는 class/pool/CPU/memory 네 blocker를 동시에 만든 뒤 pool release→CPU release→memory reconcile→class release를 적용하고 매 단계 full blocker set, primary, queue conservation, 마지막 단일 promotion을 검증한다. 실행 판정은 최종 committed HEAD의 receipt에만 둔다.
- 모두 현재 소스 결함으로 단정하지 않는다. Nightly modelcheck 미실행도 deterministic CI-profile PASS와 분리한다.

## 인계 및 중단 조건

- Do not treat arbitrary concurrent snapshots as sequential-model steps.
- Engine production changes remain serial with BG25-008 and BG25-010.
