# TM16-017 — Semgrep test-quality 규칙이 실제 integration tests를 스캔하지 않는다

- Severity: P2
- Status: OPEN
- Lane: verification-semgrep
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

Justfile:38-39; tools/semgrep/tests/test_rules_fire.py:50-63,97-103; tools/semgrep/rules/test-quality.yml

## Trigger / 관찰

정상 front door인 semgrep --config tools/semgrep/rules --verbose --error crates를 실행한다.

실제 taskmesh/engine/contract/rayon/bench integration tests가 Semgrep 기본 .semgrepignore로 skip된다. 52 skipped paths/50 scanned files. rule firing fixture는 모든 rule에 contract/src 경로만 사용한다.

## 원인 / 영향 / 범위

규칙이 trigger에서 firing하는 것과 해당 real tests가 scan에 enroll되는 것은 별개다. 현재 green Python rule test는 integration-path enrollment를 증명하지 않는다. production error-handling scans가 빠졌다는 주장은 하지 않는다.

## 보완 계획

명시적 ignore/include 정책으로 실제 tests를 enroll하고 의도된 rule 범위를 분리한다. test-quality fixture를 crates/<name>/tests에 놓고 actual front door로 skipped list/target membership까지 확인한다.

## Acceptance / 회귀 검증

실제 integration test 위치의 seeded randomness/tautology/permissive outcome trigger가 scan에서 발견되고 clean fixtures가 통과하는지 확인한다.

