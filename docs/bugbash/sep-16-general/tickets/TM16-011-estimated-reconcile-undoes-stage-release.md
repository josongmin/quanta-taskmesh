# TM16-011 — Estimated reconcile이 반환된 stage reservation을 다시 늘린다

- Severity: P2
- Status: OPEN
- Lane: engine-memory
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/features/memory/mod.rs:17-27,54-58,105-106

## Trigger / 관찰

OnStageBoundary + Estimated에서 8 units admit, release_stage_memory(p,6), reconcile_memory(p,2)를 순서대로 호출한다.

held가 8→2→8로 바뀐다. 측정 reading이나 새 allocation이 늘지 않았는데 original reserved_units를 복원한다. retained probe 재현. Measured의 동일 2-byte reading은 2 유지이고 Hybrid의 estimate floor 복원은 별도 계약으로 본다.

## 원인 / 영향 / 범위

stage release는 effective_units만 줄인다. Estimated effective_units 함수는 불변 original reserved_units를 반환한다. 따라서 'Estimated reconcile accounting-neutral'와 stage release가 합성되지 않는다.

## 보완 계획

outstanding estimate를 original estimate와 분리하거나 Estimated reconcile을 현 effective ledger에 대해 no-op으로 만든다. 새 reservation 증가가 필요하면 explicit reserve/grow API로 분리한다.

## Acceptance / 회귀 검증

release 이후 반복 reconcile, queued promotion, root/class/global 합, Estimated/Measured/Hybrid matrix를 독립 기대값으로 검증한다.

