# TM16-025 — Prompt-manager의 실제 target이 SSOT 선언과 다르다

- Severity: P3
- Status: OPEN
- Lane: quality-docs
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

tools/pm/README.md:1-42; tools/pm/targets.yaml; tools/pm/pm.py:cmd_lint; Justfile:44-48,84

## Trigger / 관찰

`uv run python tools/pm/pm.py lint`를 실행한다.

exit 1: AGENTS.md drift, CLAUDE.md/.codex/CODEX-RULES.md/.codex/CODEX-START-PROMPT.md/.cursorrules missing.

## 원인 / 영향 / 범위

README는 generated agent/operator docs의 source of truth라고 선언하지만 실제 checked-in target 상태와 일치하지 않는다. fixture 기반 pm unit test가 green이어도 실제 repo target drift를 검증하지 않는다. logq 명칭/다른 workflow scaffold residue도 남아 있다. Runtime 결함은 아니다.

## 보완 계획

PM을 실제 owner로 유지할지 retire할지 먼저 정하고 current AGENTS 정책을 보존하며 sources/targets와 정합화한다. blind `sync`로 현재 repo-specific AGENTS 규칙을 덮어쓰지 않는다. 유지한다면 real-target lint를 canonical gate에 연결하고 필요한 output만 version/ignore 정책을 정한다.

## Acceptance / 회귀 검증

선택한 supported targets 모두 real-target lint green, unit fixture green, AGENTS의 fail-closed/capability-pool/product-neutral 규칙 유지, retired targets는 명시적으로 manifest에서 제거됨을 확인한다.
