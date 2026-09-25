# BG25-010 — combined memory ledger and snapshot oracle

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P1
- 선행: BG25-008, BG25-009
- 소유: engine/memory owner, contract snapshot owner

## 목적

stage release, measured/hybrid reconcile, promotion, clock activity, leak sweep를 하나의 input-derived memory ledger로 검증하고 Snapshot helper의 제한을 명시한다.

## 근거

- epoch, stale report, stage release, live-lease sweep의 isolated tests는 강하다.
- 기존 independent checks 일부는 production `permit_ledgers()`를 재합산하므로 외부 event oracle과 완전히 독립적이지 않다.
- forged wire Snapshot은 phase/cumulative equations가 맞아도 held resource inconsistency를 포함할 수 있다.

## 변경 파일

- `crates/taskmesh-engine/tests/hardening_memory_ledger.rs`
- `hardening_memory_epochs.rs`, `hardening_snapshot_projection.rs`
- `crates/taskmesh-contract/tests/snapshot_oracle.rs`
- 재현 시 `features/memory/mod.rs`, `engine/{governor,state}.rs`, snapshot docs

## 작업 계획

1. estimate/remaining/effective, epoch, stage sequence, lease activity를 test-side ledger로 계산한다.
2. stage return→reconcile→queue promotion→sweep 순서를 barrier/ManualClock으로 고정한다.
3. actual Governor snapshot and permit-ledger projection을 independent expected values와 비교한다.
4. wire Snapshot helper와 live internal ledger proof를 문서상 분리한다.

## DoD

- `BG25-010-H10`: returned units never resurrect, only ordered measurement applies, live lease is never swept, released capacity promotes exactly once.
- `BG25-010-D20`: forged held/occupancy mismatch demonstrates helper scope; live Governor state is checked against independent event ledger and policy inventory.

## 검증

- Focused engine memory and contract snapshot tests.
- Reconciliation callback/promotion history includes exact final zero and cumulative counters.

## 인계 및 중단 조건

- `conservation_violation()` or `permit_ledgers()` alone cannot satisfy independent oracle DoD.
- Shared Governor/state edits remain single-writer after BG25-009.
