# BG25-011 — simulator and real-host load evidence

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P1
- 선행: BG25-005, BG25-006, BG25-008
- 소유: benchmark owner; host fixture has host owner

## 2026-09-25 재감사 잔여

- **정확성:** H17/H28의 simulator·실제 host 모집단 fixture가 직전 clean HEAD `da5356b`의 `test`·`bench-smoke` 게이트에서 PASS였다. 새 확정 결함 없음.
- **조건부 성능 증거:** latency/throughput 개선·회귀를 주장할 때만 동일 source·toolchain·feature·workload·seed·quiet-host baseline과 응답/worker 분모를 기록한다. 현 baseline 부재는 정확성 실패가 아니다.
- **남은 기본 증거:** 문서 변경 후 clean exact-source CI 재검증은 BG25-012가 소유한다.

## 목적

Simulator admission-wait metrics와 실제 public host의 end-to-end response/custody를 분리하고 각 모집단의 denominator를 보존한다.

## 근거

- `taskmesh-bench` load generator is a simulator, not Tokio facade execution.
- `hellgate::multiclass_multiseed_population_has_no_unaccounted_request`가 multi-class/seed simulator denominator를 검증한다.
- `host_open_loop::offered_terminal_unanswered_and_execution_counts_close_exactly`가 public host의 응답·미응답 및 worker custody를 독립 집계한다.
- allocation/instruction gates are not latency qualification.

## 변경 파일

- `crates/taskmesh-bench/src/{loadgen,metrics,workload}.rs`
- `crates/taskmesh-bench/tests/{hellgate,inferno,fairness_property}.rs`
- host fixture `crates/taskmesh/tests/host_open_loop.rs`
- performance policy/evidence docs; no threshold change without baseline evidence

## 작업 계획

1. Simulator multi-class/multi-seed ledger records offered, immediate admit, promoted, completed, rejected, leftover.
2. Host workload uses bounded producer rate and records every terminal response plus unanswered and surviving custody.
3. Separate admission wait, caller response latency, and worker termination latency.
4. Record machine, source, toolchain, features, seed, warmup, sample count, and quiet-host conditions.

## DoD

- `BG25-011-H17`: every simulator request belongs to one final category and completed/promoted counts agree across classes/seeds.
- `BG25-011-H28`: real host records `offered = terminal + unanswered`; post-response custody is reconciled separately and simulator metrics are never labelled host latency.

## 검증

- Deterministic correctness fixtures는 최종 committed HEAD의 BG25-012 receipt에서 판정한다.
- Performance numbers require a separate quiet-host baseline; first baseline is not a regression PASS.

## 최종 의미 감사

- H28은 `taskmesh-bench/tests/host_simulator_comparison.rs::real_host_and_simulator_agree_on_bounded_burst_accounting`으로 매핑한다. 동일 burst에서 host/simulator의 offered/completed/rejected/max-queue와 host 실행 횟수를 대조한다. `host_open_loop`의 별도 response/worker custody case는 유지됐다.
- 새 case는 virtual wait를 host latency로 해석하지 않고 동일 9-request burst에서 host와 simulator의 offered/completed/rejected/max-queue를 직접 비교한다. 다중 class/path 성능, warmup/환경 기록, quiet-host latency 비교는 성능 qualification의 별도 범위이며 production 결함은 확인되지 않았다.

## 인계 및 중단 조건

- Do not add long timing campaigns to `just dev` or ordinary CI.
- Any runtime file change is handed back to BG25-005/006 owner.
