# TM16-033 — Test-quality 규칙이 존재하지 않는 mutation/coverage gate를 보완책으로 안내한다

- Severity: P3
- Status: OPEN / documentation and gate-inventory drift
- Lane: G/Q — structural validation / documentation
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `tools/semgrep/rules/test-quality.yml:3-4`: L1–L5 cheese detector, `just mutants-critical`, `just cov-gate`, Justfile cheese detector section을 안내한다.
- 같은 파일 `:153-160`: assertion-free proptest 규칙을 구현하지 않는 이유로 L3 mutation gate가 검출한다고 설명한다.
- 현재 Justfile에는 해당 recipe/section이 없고 CI workflow에도 cargo-mutants/coverage gate가 없다.

## Trigger / 관찰

`just --show mutants-critical`, `just --show cov-gate`가 각각 exit 1, `justfile does not contain recipe`로 실패한다. 해당 이름/cargo-mutants에 대한 repository 검색은 test-quality rule comments 외의 구현을 찾지 못했다.

## 원인 / 영향 / 범위

복사된 policy 설명이 현재 gate inventory와 연결되지 않는다. Semgrep이 intentionally 검사하지 않는 영역을 다른 도구가 실제로 커버한다는 잘못된 설명이다. runtime bug나 mutation testing을 제품이 반드시 지원해야 한다는 판정은 아니다. TM16-006의 존재하는 local/CI gate parity, TM16-017의 실제 test scan 누락과는 별도의 documentation defect다.

## 보완 계획

- mutation/coverage를 실제 채택할지 결정한다. 채택하지 않으면 없는 command와 coverage 주장을 제거하고 uncovered 영역을 정확히 표시한다.
- 채택한다면 reproducible tool version, eligible source inventory, negative fixtures, local/CI front door를 함께 등록한다.
- policy에 등장하는 recipe names가 실제 `just --list` inventory에 존재하는지 drift 검증을 추가한다.

## Acceptance / 회귀 검증

- 정책이 존재하지 않는 command나 phantom verification coverage를 안내하지 않는다.
- 도구를 추가한 경우 assertion 제거 mutant가 실제 gate를 red로 만들고, 단순 도구 설치/실행 성공을 coverage proof로 세지 않는다.
