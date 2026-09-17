# TM16-003 — Fallible Builder가 잘못된 topology에서 panic한다

- Severity: P2
- Status: OPEN
- Lane: contract-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-contract/src/topology.rs:137-175; crates/taskmesh/src/builder.rs:84-110; crates/taskmesh/src/runtime.rs:178-186,894-912

## Trigger / 관찰

`TopologyConfig::new().min_workers(8).max_workers(2)`를 Builder에 주입해 build한다.

build가 Err 대신 `min > max. min = 8, max = 2` panic을 발생시킨다. 기본 executor와 Rayon의 resolver 모두 같은 clamp를 호출한다.

## 원인 / 영향 / 범위

topology는 public/serde 입력인데 construction-time validation이 없다. 큰 slot count는 Semaphore 허용 최대보다 크면 별도 panic도 유발할 수 있으므로 함께 검증해야 한다.

## 보완 계획

TopologyConfig validation을 단일 함수로 만들고 min/max, 실제 Semaphore maximum, worker count의 OS/runtime 한계를 build 전에 검증한다. fallible Rayon topology constructor도 검증 오류를 표현하도록 정리한다.

## Acceptance / 회귀 검증

min>max, min/max 0의 선언된 의미, 과도한 slot count, 정상 Auto/Fixed/reserve 조합, default/rayon 두 feature에서 panic 없는 Err 반환을 확인한다.

