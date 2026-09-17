# TM16-006 — CI와 just gate의 정책 집행 표면이 다르고 현재 local gate가 실패한다

- Severity: P2
- Status: OPEN
- Lane: verification-ci
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

Justfile:26-39,79-95; .github/workflows/ci.yml:19-37; .github/workflows/bench.yml:11-82; crates/taskmesh/src/runtime.rs:114,163,707

## Trigger / 관찰

현재 HEAD에서 just clippy, cargo deny check, semgrep --config tools/semgrep/rules --error crates를 실행한다.

just clippy는 runtime.rs:707 needless_pass_by_value로 exit 101. semgrep은 tx.send 결과 discard 2건으로 exit 1. cargo deny는 TM16-007의 두 advisory로 exit 1. 기본 CI clippy는 pedantic/nursery/restriction config를 사용하지 않는다. 두 workflow 모두 deny/semgrep/architecture/python gate를 실행하지 않는다.

## 원인 / 영향 / 범위

GitHub 기본 proof가 green이어도 저장소가 약속한 fast gate와 full proof green을 증명하지 못한다. bench.yml의 allocation/loom/shuttle는 실제로 wired되어 있으므로 '벤치 gate 전체 누락'으로 확대하지 않는다. 실제 branch protection required-check 설정은 이번 local audit에서 조회하지 않았다.

## 보완 계획

CI가 just의 동일 canonical gate를 호출하거나 공통 명령 파일에서 생성되도록 한다. lint를 고치고 예상된 receiver-drop send 실패에는 정책이 정한 same-line reason을 명시해 정리한다. Python unit test 성공과 pm real-target drift proof를 구분한다.

## Acceptance / 회귀 검증

CI와 local recipe 명령 parity, 의도적 rule violation fixture가 각 required gate를 red로 만드는지, 정상 receiver drop에서 worker lease가 회수되는지 확인한다.

