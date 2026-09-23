# TM16-020 — Local IAI proof threshold와 CI baseline compatibility가 일치하지 않는다

- Severity: P2
- Status: OPEN
- Lane: verification-benchmark
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

tools/bench-iai.sh:19; .github/workflows/bench.yml:25-29,49-62; Justfile:92-95

## Trigger / 관찰

기본 환경의 just release/bench-iai를 실행하거나 benchmark/dependency hash가 바뀐 CI cache miss에서 restore prefix를 사용한다.

CI만 IAI_CALLGRIND_REGRESSION=Ir=5.0을 설정한다. local script에는 threshold가 없고 bench code에도 RegressionConfig가 없다. cache key에 compatibility hash가 있지만 restore-keys는 hash를 제거하여 다른 bench/deps baseline을 가져올 수 있다.

## 원인 / 영향 / 범위

'full proof matches CI'가 기본 local invocation에는 성립하지 않는다. fresh first-run 비교 skip 자체는 documented behavior이므로 결함으로 세지 않는다. runner 0.14.2 primary source args.rs는 env regression 옵션을 지원함을 확인했다. Linux/valgrind regression execution은 미실행이다.

## 보완 계획

qualification용 threshold/config를 단일 canonical source로 공유한다. baseline에 bench definition/deps/runner metadata를 저장하고 mismatch면 warm cache와 qualifying comparison을 구분한다. 문서상 모든 이벤트 gate가 아니라 실제 Ir gate 범위를 설명한다.

## Acceptance / 회귀 검증

Linux에서 known >5% regression의 local/CI 동일 nonzero, compatible restore 성공, bench/dependency/runner mismatch의 qualification 거부, fresh baseline의 explicit nonqualified outcome을 검증한다.
