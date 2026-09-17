# TM16-015 — Requested-stack 결과 반환에 lease release fence가 없다

- Severity: P2
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh/src/runtime.rs:78-80,114-130,157-174,604-608; crates/taskmesh/tests/deadline_cancel.rs:98

## Trigger / 관찰

oneshot send 직후 receiver가 worker의 _lease drop보다 먼저 resume한다.

requested-stack async/blocking result가 반환됐지만 immediate snapshot에 해당 inflight가 남을 수 있다. 한 slot의 sequential zero-timeout submission도 이전 slot release와 race한다. conservative accounting이며 capacity undercount가 아니다. forced scheduling 재현은 미실행, source interleaving 근거다.

## 원인 / 영향 / 범위

CPU는 outcome와 lease를 함께 전달해 caller에서 drop하지만 requested-stack은 outcome만 보낸다. worker runtime/context teardown과 성공 응답의 release 순서가 다르다. 테스트는 반환 직후 inflight=0을 이미 기대한다.

## 보완 계획

owned runtime/context teardown 이후 (outcome,lease)를 channel로 transfer하고 receiver가 lease를 drop한 뒤 반환한다. receiver가 gone이면 worker에서 failed send payload를 drop하여 ownership을 보존한다.

## Acceptance / 회귀 검증

worker result-send 뒤 pause hook으로 interleaving을 강제한다. 두 경로의 immediate snapshot, sequential ZERO acquire, task error/panic/caller drop에서 lifetime과 release를 검증한다.

