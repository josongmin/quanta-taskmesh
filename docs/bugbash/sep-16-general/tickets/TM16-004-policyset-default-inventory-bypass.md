# TM16-004 — PolicySet의 public Default/fields가 builtin inventory 권위를 우회한다

- Severity: P2
- Status: OPEN
- Lane: engine-inventory
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/shared/mod.rs:59-88; crates/taskmesh-engine/src/engine/governor.rs:37-39,353-469; crates/taskmesh-engine/tests/audit_hardening.rs:150-169

## Trigger / 관찰

`PolicySet::default()`에 정상 class를 추가한 뒤 Governor::new를 호출한다. 또는 PolicySet::new의 public substrates를 clear/교체한다.

빈 inventory로 Governor가 성공적으로 생성되고 작업을 admit한다. snapshot.substrates도 빈 배열이다. 주석의 'built-ins are intrinsic to any governor'와 불일치한다.

## 원인 / 영향 / 범위

canonical constructor만 inventory를 seed한다. validate_policy는 registry records 자체를 검증하지 않으며 builtin 존재/정확한 kind/pool 및 map key==record.name도 강제하지 않는다. public field를 통해 register의 검증을 우회할 수 있다.

## 보완 계획

Default를 canonical new에 위임하거나 제거한다. PolicySet의 validated 필드를 private으로 제한하고 Governor::new에서 registry 전체 불변식(필수 builtin과 canonical binding)을 확인한다. 임의 direct embedder policy의 허용 범위를 문서화한다.

## Acceptance / 회귀 검증

default/struct construction/clear/잘못된 key/kind/pool 경로를 검증하고 일반 Builder와 direct Governor inventory가 같은지 확인한다.
