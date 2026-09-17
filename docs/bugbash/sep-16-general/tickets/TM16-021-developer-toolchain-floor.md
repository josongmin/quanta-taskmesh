# TM16-021 — Declared Rust floor와 locked developer proof graph가 호환되지 않는다

- Severity: P2
- Status: OPEN
- Lane: verification-msrv
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

Cargo.toml:15-16; Cargo.lock:getrandom/proptest/clap; .github/workflows/ci.yml:14

## Trigger / 관찰

cargo +1.83.0 check -p taskmesh-engine --test prop_invariants --locked (선언 floor 1.81보다 새 Cargo).

main/subagent에서 exit 101: getrandom 0.4.2 edition2024 manifest parse 실패. locked getrandom/proptest/clap는 rust-version 1.85를 요구한다. subagent production-only taskmesh lib는 1.83에서 check 성공했다. 실제 1.81 consumer MSRV는 미검증이다.

## 원인 / 영향 / 범위

workspace rust-version=1.81이 consumer/API floor인지 개발 gate까지 포함하는지 분리되어 있지 않다. stable-only CI는 floor drift를 검출하지 못한다. 이 지적은 dev/bench proof graph이고 production consumer 빌드 실패를 뜻하지 않는다.

## 보완 계획

consumer MSRV와 developer/benchmark toolchain floor를 별도로 선언하고 각각 locked minimal feature matrix를 qualify한다. dev deps를 floor에 맞춰 pin할지 dev floor를 올릴지 선택한다.

## Acceptance / 회귀 검증

선언 consumer floor의 default/rayon lib check, declared dev floor의 focused integration/bench check, 안정판 전체 gate를 CI matrix로 확인한다.

