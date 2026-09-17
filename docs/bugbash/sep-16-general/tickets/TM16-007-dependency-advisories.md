# TM16-007 — Locked dependency supply-chain gate가 red다

- Severity: P2
- Status: OPEN
- Lane: dependency
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

Cargo.lock:256-264,proc-macro-error2 package; config/deny.toml:5-17; crates/taskmesh-rayon/Cargo.toml; crates/taskmesh-bench/Cargo.toml

## Trigger / 관찰

cargo deny check --config config/deny.toml

`crossbeam-epoch 0.9.18` RUSTSEC-2026-0204 및 `proc-macro-error2 2.0.1` RUSTSEC-2026-0173로 advisories FAILED. crossbeam은 Rayon 경로에도 포함된다. proc-macro는 iai-callgrind macro의 bench tooling 의존이다.

## 원인 / 영향 / 범위

crossbeam advisory의 취약 동작은 Atomic/Shared invalid pointer formatting이다. taskmesh가 그 취약 formatting을 실행한다는 재현은 없으며 transitive inclusion을 직접 exploit 증거로 취급하지 않는다. proc-macro advisory는 unmaintained이며 vulnerability와 구분한다.

## 보완 계획

crossbeam-epoch를 >=0.9.20으로 갱신하고 Rayon graph/tests를 검증한다. iai-callgrind macro chain은 maintained 대체/업데이트를 확인하거나 dev-only 영향과 제거 계획/기한을 가진 explicit exception을 검토한다. blanket ignore로 gate를 녹색화하지 않는다.

## Acceptance / 회귀 검증

deny all-features green, cargo tree -i 각 dependency, default/rayon feature tests, benchmark compile 및 iai runner version parity를 검증한다.

