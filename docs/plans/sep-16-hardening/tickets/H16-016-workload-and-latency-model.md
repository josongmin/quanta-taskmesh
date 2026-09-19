# H16-016 — Validated workload·MMPP·raw latency

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: B — Benchmark owner (실제 assignee 미지정)
- 선행 완료: [H16-015](H16-015-benchmark-measurement.md)
- 원본 finding: [TM16-027](../../../bugbash/sep-16-general/tickets/TM16-027-invalid-trace-times-not-rejected.md), [TM16-028](../../../bugbash/sep-16-general/tickets/TM16-028-mmpp-crosses-phases-with-old-rate.md), [TM16-029](../../../bugbash/sep-16-general/tickets/TM16-029-open-loop-double-corrects-latency.md)
- 배타적 write lease: `bench`; [적용 순서](EXECUTION.md) 준수

## 목적

parser뿐 아니라 직접 API 입력·가상 시간·MMPP event scheduling·histogram population을 검증한다.

## 변경 범위

- 기존: [crates/taskmesh-bench/src/workload.rs](../../../../crates/taskmesh-bench/src/workload.rs)
- 기존: [crates/taskmesh-bench/src/loadgen.rs](../../../../crates/taskmesh-bench/src/loadgen.rs)
- 기존: [crates/taskmesh-bench/src/metrics.rs](../../../../crates/taskmesh-bench/src/metrics.rs)
- 기존: [crates/taskmesh-bench/examples/loadgen_probe.rs](../../../../crates/taskmesh-bench/examples/loadgen_probe.rs)
- 기존: [crates/taskmesh-bench/examples/usl_probe.rs](../../../../crates/taskmesh-bench/examples/usl_probe.rs)
- 제안 경로: `crates/taskmesh-bench/tests/workload_reference.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] ValidatedArrivals/SimulationConfig를 만들어 finite/nonnegative/nondecreasing time, ns 변환 범위, t+service overflow를 validate한다. raw slice 직접 simulate 경로도 검증을 우회하지 못한다.
- [x] rate/dwell/count/classes/Zipf 설정을 명시적으로 검사한다. zero/nonfinite dwell과 phase_end nonprogress는 loop 진입 전 또는 typed numeric failure로 종료한다.
- [x] MMPP는 arrival와 phase boundary 중 앞선 사건을 처리한다. integrated hazard/CTMC reference와 rate·occupancy·multiple seed 결과를 비교한다.
- [x] raw open-loop latency는 admitted/started request당 한 sample만 기록한다. synthetic correction은 별도 타입/모집단/metric label로 분리한다.
- [x] simulation leftovers와 offered conservation, queued terminal 정리를 검사한다. zero observations/undefined tail ratio를 'perfect fairness/flat tail'과 구별해 표시한다.
- [x] USL의 no finite peak를 near-linear로 자동 표시하지 않는다. alpha/beta/domain/empirical peak와 fit 실패 원인을 구분한다.
- [x] generator/trace schema version과 seed를 보존한다. generator 수정 후 기존 trace를 새 생성 알고리즘 결과로 오인하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `ValidatedArrivals`: finite/nonnegative/nondecreasing/`MAX_SEND_TIME_SECS`(2^53 ns) 검증; simulate는
  이 타입만 받으므로 raw slice 우회 불가. `trace_from_csv`는 위반 line 번호를 반환.
- `WorkloadConfig::validate` / `BurstConfig::validate`: rate/dwell/zipf finite positive; zero-dwell은
  loop 진입 전 typed 오류(bounded time). `NoProgress` 방어 bound 추가.
- MMPP: arrival와 phase boundary 중 앞선 사건을 처리하고 boundary에서 새 rate로 재추출
  (memoryless라 정확). `BurstConfig::stationary_rate()`가 독립 CTMC reference. 4 seed, 8% tolerance.
  `GENERATOR_VERSION = 2`.
- `LatencyRecorder`: `Raw` / `OmissionCorrected` 명시 mode. simulate는 Raw이며 started당 1 sample;
  `synthetic_samples()`로 population 분리.
- checked `t + service_ns` → `CompletionTimeOverflow`.
- USL: β≤0은 "no finite peak"로 보고, "near-linear" 자동 표기 제거(Q25).

Mutations KILLED: `mmpp-draws-at-the-old-rate`, `reversed-trace-accepted`,
`open-loop-histogram-double-corrected`.

## 검증 / 완료 조건

- [x] `H16-016-A01` NaN/inf/negative/reversed/overflow가 CSV·direct 모두 reject
- [x] `H16-016-A02` zero-dwell은 bounded time에 typed 실패; 정상 control 완료
- [x] `H16-016-A03` equal dwell low1/high1000이 500.5/s ± 8% (4 seeds)
- [x] `H16-016-A04` histogram len == started; synthetic 0
- [x] `H16-016-A05` offered = completed + rejected + leftover (기존 `is_conserved` 유지)
- [x] `H16-016-A06` USL saturation-without-peak 정확 report

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-bench --test hellgate --test inferno
cargo test -p taskmesh-bench --lib
```

## 호환성 / 실패 모드

- 여러 통계적 seed를 사용하되 fixed magic expected sample count로 알고리즘을 맞추지 않는다.
- valid input의 progress와 malformed input 실패를 별도 acceptance로 둔다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

