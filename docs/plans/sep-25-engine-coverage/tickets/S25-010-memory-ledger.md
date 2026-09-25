# S25-010 — memory release·reconcile 독립 원장

- 상태: PLANNED; 우선: P1; 선행: [S25-008](S25-008-admission-capacity.md), [S25-009](S25-009-queue-races.md); 소유: engine/memory owner, contract snapshot owner.
- 주 담당 시나리오: H10, D20.
- 근거: `crates/taskmesh-engine/src/features/memory/**`, `hardening_memory_epochs.rs`, `hardening_exact_accounting.rs`, `crates/taskmesh-contract/src/snapshot.rs`.

## 목적

입력 event에서 expected-held를 직접 계산하는 **테스트 전용 독립 ledger**를 둔다. Snapshot 값을 다시 합산해 Snapshot과 비교하지 않는다. stage release, measured/hybrid reconcile, promotion, active lease sweep을 한 fixture에서 순서대로 적용하고 stale/duplicate event의 no-change를 본다. wire helper의 범위와 실제 Governor permit ledger의 범위를 구분한다.

## 변경 파일

| 구분 | 파일 | 조치 |
|---|---|---|
| 신규 fixture | crates/taskmesh-engine/tests/hardening_memory_ledger.rs | offer·stage release·reconcile·sweep·promotion 입력 이벤트로 expected held/epoch/lease를 독립 계산하는 combined history. |
| 기존 회귀 | crates/taskmesh-engine/tests/hardening_memory_epochs.rs, hardening_exact_accounting.rs, memory_modes.rs, memory_overcommit.rs, leak_sweep.rs; crates/taskmesh-contract/tests/snapshot_oracle.rs | mode별 단독 회귀 유지, 위조 Snapshot negative fixture 추가. |
| 조건부 source | crates/taskmesh-engine/src/features/memory/mod.rs, engine/governor.rs, engine/state.rs | 재현된 effective units·sequence·lease·promotion 불일치만 수정. S25-009와 같은 Governor 파일은 순차 적용. |
| 조건부 wire/docs | crates/taskmesh-contract/src/snapshot.rs; docs/taskmesh-library-spec.md, taskmesh-external-interface.md | helper 범위 문구 정정. Snapshot field/version 변경은 S25-001/003 계약 결정 후 contract owner만 적용. |

## 구현 순서

1. Estimated/Measured/Hybrid별 expected effective-held와 stage release policy를 입력 계약표로 고정한다. 테스트용 원장은 permit별 reservation, released units, latest accepted sequence, measured bytes, active worker flag를 직접 유지한다. production memory helper와 ledger effective_units를 기대값 계산에 사용하지 않는다.
2. 두 class에서 admit→stage release→stale/duplicate/latest reconcile→memory-blocked queue→active worker sweep→holder release→promotion/claim을 하나의 history로 수행한다. 매 단계의 expected-held·class/global aggregate·phase·ticket를 비교한다.
3. sequence 끝값, unknown permit, zero-unit release, stale lease와 active worker를 분기한다. 적용 거절 이벤트의 held/epoch 불변을 확인하되 zero-unit release는 현행 activity 계약대로 처리한다.
4. phase/cumulative 등식과 capability limit이 맞지만 held 또는 inventory가 거짓인 Snapshot을 만들어 helper가 증명하지 못하는 범위를 명시한다. Governor의 진짜 snapshot은 permit별 입력 원장 및 policy/inventory로 다시 합산한다.
5. 반례가 나면 최소 memory transition만 수정한다. promotion/effect 계약은 S25-009와 재확인하고 public Snapshot 확장은 별도 호환성 결정에 맡긴다.

## DoD

- [ ] `S25-010-H10` stage 반환 단위가 측정으로 부활하지 않고 epoch/sequence가 한 번씩만 적용되며 active worker lease를 sweep이 회수하지 않는다. queued promotion과 held 합계를 독립 원장으로 확인한다.
- [ ] `S25-010-D20` phase/누적 등식이 맞지만 held/inventory가 거짓인 조작 Snapshot에서 helper의 한계를 재현한다. 실제 Governor `permit_ledgers()`와 policy inventory로 재계산한 held/pool/class 총량만 내부 원장 합격으로 인정한다.

각 항목의 추가 판정:

- H10: stage 반환 단위가 추후 reconcile에서 부활하지 않는다. 낮거나 같은 sequence는 typed no-op이고 held/epoch가 변하지 않는다. 최신 측정은 mode별 기대값으로 한 번 적용되며 memory 반환 뒤 queued ticket이 정확히 한 번 승격된다. active worker lease는 stale이어도 suspect만 하고 reclaim하지 않는다. 최종 held·queue는 0이다.
- D20: 위조 Snapshot이 conservation helper를 통과할 수 있는 한계를 fixture로 보여 준다. 실제 Governor snapshot의 CPU/memory/class/pool 합계는 입력 event oracle 및 permit_ledgers()의 exact u128 합산과 맞아야 한다. helper의 None 반환만으로 resource correctness를 승인하지 않는다.

## 계획된 검증

- Owner-local: cargo test --locked -p taskmesh-engine --test hardening_memory_ledger; cargo test --locked -p taskmesh-engine --test hardening_memory_epochs; cargo test --locked -p taskmesh-contract --test snapshot_oracle.
- 통합: S25-012가 같은 clean HEAD에서 just test-core/just test를 적용한다. 이 계획 작성 중 테스트는 실행하지 않는다.

## 인계·중단 조건

- 인계: mode별 계산표, event trace와 expected held/epoch, queued promotion·worker custody, Snapshot helper의 검사 범위, 실제 source 변경 필요 여부.
- permit_ledgers()가 custody를 관측하기에 부족하면 snapshot aggregate에서 기대값을 역산하지 않는다. 관측 seam을 engine owner와 설계한다. wire field/version은 S25-003 수용 전에 변경하지 않는다.
