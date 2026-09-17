# TM16-016 — Architecture checker가 미등록 crate/외부 의존성과 optionality를 누락한다

- Severity: P2
- Status: OPEN
- Lane: verification-architecture
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

tools/arch/check_crate_boundaries.py:89-132,160-170

## Trigger / 관찰

unknown workspace package, engine→Tokio, engine→bench, nonoptional host→Rayon의 cargo metadata fixture를 check_edges에 넣는다.

네 fixture 모두 [] violations다. main Python diagnostic과 subagent가 각각 재현했다. read failure는 OSError를 continue하고 필요한 source dir 부재도 green으로 반환한다.

## 원인 / 영향 / 범위

names를 WORKSPACE_CRATES와 먼저 교집합하고 dep_name도 그 known set으로 제한하므로 unknown/외부/runtime dependency가 검사 전에 사라진다. optional=True 여부는 set에 보존하지 않는다. unknown crate reject branch가 reachable하지 않다.

## 보완 계획

workspace_members와 실제 metadata inventory 전체를 검증한다. 외부 dependency 허용 정책은 engine parking_lot 등 실제 정상 graph를 반영해 explicit하게 만든다. feature optionality/rename/kind를 metadata로 확인하고 required source read failure는 reject한다.

## Acceptance / 회귀 검증

네 negative fixture, known good graph, missing/unreadable source, renamed and target-specific deps, malformed metadata에서 fail-closed를 검증한다.

