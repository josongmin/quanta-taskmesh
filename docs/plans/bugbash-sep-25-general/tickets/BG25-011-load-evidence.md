# BG25-011 — simulator and real-host load evidence

- 상태: PLANNED
- 우선순위: P1
- 선행: BG25-005, BG25-006, BG25-008
- 소유: benchmark owner; host fixture has host owner

## 목적

Simulator admission-wait metrics와 실제 public host의 end-to-end response/custody를 분리하고 각 모집단의 denominator를 보존한다.

## 근거

- `taskmesh-bench` load generator is a simulator, not Tokio facade execution.
- current hellgate load tests are primarily single-class.
- host chaos tests submit fixed workloads but do not independently count offered/terminal/unanswered and post-response worker custody.
- allocation/instruction gates are not latency qualification.

## 변경 파일

- `crates/taskmesh-bench/src/{loadgen,metrics,workload}.rs`
- `crates/taskmesh-bench/tests/{hellgate,inferno,fairness_property}.rs`
- 신규 host fixture candidate `crates/taskmesh/tests/host_open_loop.rs`
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

- Deterministic correctness tests remain default gate candidates.
- Performance numbers require a separate quiet-host baseline; first baseline is not a regression PASS.

## 인계 및 중단 조건

- Do not add long timing campaigns to `just dev` or ordinary CI.
- Any runtime file change is handed back to BG25-005/006 owner.
