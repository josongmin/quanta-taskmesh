# B07 — 필요한 recovery / soak 안정성 검증

- 상태: **PLANNED — 구현·실행 증거 없음**
- 기준 소스: `5be7e0995f5f46fe3174f8983c5ec5773ea7b286` (2026-09-27)
- 우선순위: 안정성 coverage 보완. 현재 확인된 엔진 결함이나 release blocker로 분류하지 않는다.
- 선행: [B04](B04-remaining-audit.md)의 실행·metadata·artifact 경계 보완은 구현됨.
- 결정: 모든 모드의 성능 admission 확장 대신, **같은 host의 반복 과부하·회복·자원 반환**만 추가한다.

## 목적과 완료 범위

과부하 후 정상 작업이 다시 실행되는지, caller 응답 이후 남은 worker가 실제로
종료되고 capacity가 반환되는지, 이 과정이 같은 host에서 반복되어도 누적 상태가
남지 않는지 확인한다. 기존 typed raw validator와 custody oracle을 재사용한다.

B07 완료는 이 안정성 검증의 구현과 해당 범위의 실행 증거를 뜻한다. 전체 엔진의
모든 시나리오 감사, 생산 환경의 장기 메모리 안정성, 성능 SLO 또는 업계 비교의
완료 상태는 각각 별도로 판단한다.

### 현재 갖춘 것과 추가할 것

| 현재 소스 | 이미 확인하는 의무 | B07에서 추가할 의무 |
|---|---|---|
| `tests/host_load_integration.rs::burst_recovery_and_custody_fixtures_preserve_bounded_raw_ledgers` | H2/H5 fixture의 row 수, snapshot 수, unanswered, 최종 capacity와 drain/accounting | 같은 host의 여러 사이클과 각 회복 뒤 실제 정상 작업 성공 |
| `src/host_load.rs::RawHostRun::validate_against` | typed scenario/raw 일치, cut/settlement conservation, terminal drain 후 capacity 반환 | record/count 검증 함수를 공유하되, 중간 checkpoint와 terminal drain을 구분하는 cycle container validation |
| `src/host_load.rs::run_host_scenario_inner` | 단일 실행의 bounded producer, body/caller 기록과 drain | 현재는 호출마다 `build_runtime()`하므로, bench 내부의 window 실행부를 분리해 한 runtime을 재사용 |
| `tools/bench/process_resource.py` | 같은 PID의 CPU/RSS/thread 시계열, cadence, sample cap 및 unavailable 상태 | opt-in stress 동안 기존 수집을 재사용. RSS slope 합격선이나 정확한 high-water 판정은 추가하지 않음 |
| `host_run.py`, `host_build.py`, supervisor | 실행 파일/source/topology custody, timeout/interrupt 및 실패 artifact | stability probe에 기존 경계를 연결. 별도 build/receipt 체계를 만들지 않음 |

`src/`와 `tests/`는 이 표에서 `crates/taskmesh-bench/` 기준이다. 기존 단일 실행의
final drain이나 매번 새 프로세스를 실행한 결과를 same-host 누적 안정성 증거로 사용하지 않는다.

## 실행 시나리오

**host 생성 1회 → warmup 1회 → [bounded overload → settlement → exact checkpoint
→ 정상 canary] 반복 → 최종 drain**.

- 기본 경로는 현재 full-host 진단의 IO / blocking / CPU와 고정 class/topology다.
  다른 모드의 correctness는 기존 테스트 owner가 유지한다.
- overload는 고정된 유한 offer 집합으로 구성한다. producer cap으로 경쟁 큐를
  늘리지 않는다. 발생한 reject/not-submitted도 population에 남긴다.
- 각 사이클에서 모든 owned worker가 종료되고 class inflight/queued와 governed
  capability usage가 0으로 돌아오는지 exact snapshot과 기존 raw oracle로 확인한다.
- 회복 canary는 중간 settlement checkpoint 이후 각 사용 경로에 정상 작업을 제출해 실제 body 실행,
  성공 응답 및 자원 반환을 확인한다. zero snapshot만으로 회복 성공을 판정하지 않는다.
- caller cancel/deadline/drop은 worker 종료로 간주하지 않는다. 기존 barrier/custody
  oracle을 재사용하고, 반복 후 capacity 반환·다음 canary 성공만 새 의무로 추가한다.
- 사이클 번호, expected population, checkpoint/canary 및 같은 host의 누적 counters를
  보존한다. runtime 재생성으로 counters가 초기화되면 정상 완료로 인정하지 않는다.
- smoke는 **3회 반복**을 시작 구성으로 사용한다. 이는 상태 누적을 드러내기 위한
  짧은 correctness 구성이고 production soak 길이나 통계적 신뢰도를 의미하지 않는다.
- opt-in stress는 **10분**을 시작 구성으로 사용하되 duration/cycle 수, seed, cadence,
  총 record/snapshot/sample cap과 process timeout을 실행 전에 manifest에 고정한다.
  이 시간도 진단 설정이며 production 장기 안정성 보장 시간으로 표현하지 않는다.
- 장시간 실행에서는 사이클 raw를 파일로 flush하고 다음 사이클 전에 해제한다.
  artifact 보관 때문에 harness 메모리가 사이클마다 증가하는 결과를 엔진 leak로 오인하지 않는다.
- Rust cycle의 상대 `Instant` 시간과 controller sampler의 monotonic 시간을 직접
  빼서 phase latency를 만들지 않는다. resource는 같은 PID의 전체 실행 관측으로 남긴다.

## 구현 순서와 파일

### B07.1 — 같은 host의 짧은 회복 회귀

- `crates/taskmesh-bench/src/host_load.rs`: 기존 window 실행 로직을 bench 내부에서
  분리한다. 기존 단일 실행 entrypoint는 동일한 동작을 유지하고, 새 loop가 runtime을
  한 번 만들고 재사용한다. public Taskmesh API·executor pool·class policy 변경은 없다.
- **drain 경계:** `crates/taskmesh/src/runtime/drain.rs::TokioRuntime::drain`은
  `governor.close_admission()`을 호출하는 one-way 동작이다. 중간 사이클에서는
  producer를 끝내고 caller/body 작업 및 owned capacity가 반환될 때까지 명시적
  deadline 안에서 기다린다. `is_draining() == false`인 exact checkpoint를 확인한
  뒤 canary를 실행한다. `drain()`은 마지막 canary까지 끝난 뒤 한 번만 호출한다.
  기존 runtime의 admission을 다시 여는 API나 임시 정책을 추가하지 않는다.
- `crates/taskmesh-bench/src/host_stability.rs` **신규 예정** 및 `src/lib.rs`:
  typed cycle manifest/result와 순서·population·checkpoint/canary 검증만 추가한다.
  cycle record는 기존 `RawHostRecord`/`CallerCounts` 등을 재사용하고 timeline,
  offer 일치 및 count 검사 함수는 `host_load.rs`에서 공유한다. `RawHostRun`의
  terminal `drain_ok` 의미와 기존 schema는 유지한다. 중간 checkpoint를
  `drain_ok=true`인 단일 실행 raw로 위장하지 않는다.
- `crates/taskmesh-bench/tests/host_stability_integration.rs` **신규 예정**:
  같은 host 3회 반복, recovery canary, 중간 자원 반환 및 실패 경계를 검증한다.
- H2/H5의 `tools/bench/scenarios/h2-burst-recovery-smoke.json`,
  `h5-custody-smoke.json`을 각각 별도의 same-host study에서 반복한다. 한 study의
  class/policy/topology는 고정한다. 두 fixture는 각자 원래의 oracle을 통과해야 하며,
  서로 다른 workload를 같은 성능 cohort로 묶지 않는다.
- 기존 `host_load_accounting.rs`, `host_load_integration.rs`는 factoring이 기존
  row/settlement 의미를 바꾸지 않았는지 확인하는 owner regression이다.

**DoD:** 실제 1회 host 생성과 지속된 counters를 확인; 매 사이클 record/accounting 검증,
exact zero-owned-capacity checkpoint와 실제 canary 성공; 하나라도 누락/실패하면
PASS 없음. 중간 admission은 열려 있고 최종 drain 뒤에는 닫혀 있음을 확인한다.
기존 단일 실행 회귀와 `RawHostRun` terminal drain 의미도 유지한다.

### B07.2 — opt-in 장시간 진단과 artifact 연결

- `crates/taskmesh-bench/examples/host_stability_probe.rs` **신규 예정**:
  같은 host loop, create-only cycle artifact, 중단·실패 시 이미 실행한 사이클과
  실패 원인을 보존한다. cycle별 감독 프로세스 재시작으로 대체하지 않는다.
- `tools/bench/host_stability.py` **신규 예정**: manifest 확인, 기존 build/retained
  executable/owned supervisor/resource sampler 연결, 모든 expected cycle 회계와
  typed validator 실행, 진단 report 작성. 새로운 통계적 성능 admission은 없다.
- `host_build.py` / `host_run.py`: example 허용 목록과 재사용 가능한 build/custody
  경계만 필요한 만큼 연결한다. 기존 measured admission은 바꾸지 않는다.
- `tools/bench/tests/test_host_stability.py` **신규 예정**: 빠진/중복/재정렬 cycle,
  잘못된 raw/checkpoint/canary, deadline/interrupt, cap 도달 및 provenance 불일치.
- `process_resource.py`: 원칙적으로 기존 구현 재사용. 실제 연결에서 확인된 결함이
  있을 때만 해당 owner를 수정하고, CPU/RSS/thread samples의 현재 의미를 유지한다.

**DoD:** source/scenario/manifest/features/topology/binary/PID와 각 raw digest가
연결됨; 모든 expected cycle의 terminal 상태 존재; 미종료 worker, 실패한 canary,
conservation 위반 또는 실행 timeout은 functional FAIL; 누락·위조·불일치 증거는
REJECTED. 샘플이 없거나 cap에 도달하면 resource 결과는 unavailable로 남긴다.
정확한 lifecycle 증거가 유효하면 functional 결과는 별도로 판정할 수 있으나,
RSS 누적이 없었다는 결론이나 전체 성능 QUALIFIED를 생성하지 않는다.

### B07.3 — 운영 경로와 문서

- `docs/benchmarks/host-series.md`, `docs/adr/9000-benchmark-strategy.md`,
  scenario `index.json`: 구현 후 실제 지원 상태와 명령·claim 범위를 기록한다.
- 짧은 smoke는 기존 Rust test 및 Python test 경로에 포함한다. 중복 gate를 추가하지 않는다.
- 장시간 stress는 명시적 local opt-in으로 시작한다. 자동 CI/nightly 편입은 실제
  실행 비용과 안정성을 확인한 뒤 별도 결정한다. gate가 필요해지면 `Justfile`,
  `tools/gates/inventory.json`, `required.json`의 해당 owner를 함께 업데이트한다.

**DoD:** 문서가 smoke / stress / performance evidence를 구분; 실패 artifact와
중단 처리 방법이 재현 가능; 기본 daily CI에 10분 실행을 자동 추가하지 않는다.

## 누락·중복을 막는 검증 표

| 의무 | owner / 결과 |
|---|---|
| 단일 실행 cut/settlement, reject, caller/worker 구분 | 기존 raw validator와 H2/H5·custody tests 재사용 |
| 같은 host 재사용, 3회 이후 정상 진입과 반환 | 새 integration regression; builder 반복 생성 또는 counters reset 거부 |
| terminal drain과 중간 quiescence 혼동 | 중간 `is_draining()==false` + canary 성공; 마지막 `drain()` 후 기존 RuntimeUnavailable 거부 oracle 재사용 |
| 아직 끝나지 않은 blocking worker | 기존 barrier로 상태를 고정; 반환 전 zero checkpoint를 인정하지 않고 release 후 정상 복귀 확인 |
| 중간 cycle 실패·누락·중복·순서 변경 | container/CLI regression; 성공 cycle만 선택해 PASS 만드는 경로 없음 |
| 프로세스 hang·interrupt·source/binary drift | 기존 supervisor/custody 회귀 재사용 + stability 연결 검증 |
| resource unavailable / sampler cap / 빈 sample | diagnostic unavailable; leak 없음으로 승격하지 않음 |

검증 순서: 새 Rust smoke와 기존 host-load owner tests → 새 Python validator/CLI
tests → 필요 시 `verify-macos-ci`의 clean-source receipt → 별도 opt-in 실제 stress.
owner-local PASS와 clean CI, 실제 장시간 실행 결과는 각각 기록한다. 현재 이 티켓의
구현·테스트·stress 실행은 **NOT_RUN**이며 이번 변경은 계획 작성만 수행한다.

## 이번 범위에서 보류하는 항목과 재개 조건

- **모든 모드의 반복 성능 admission:** 해당 모드의 공개 성능 비교/보장 범위를
  결정하고 estimand·대조군·budget을 고정했을 때만 별도 계획으로 재개한다.
  모드별 correctness·cancel·drain·resource-return 기존 테스트 의무는 유지한다.
- **RSS slope 자동 합격선, recovery SLO 회귀율, 통계적 soak admission:** 실제
  consumer budget과 noise/allocator/harness 영향에 대한 pilot 근거가 있을 때 재개한다.
  현재의 zero-owned-capacity와 정상 canary 판정으로 메모리 leak 부재를 주장하지 않는다.
- **peer/SOTA 성능 주장:** B00, matched peer, quiet-host controls/series와 외부 재실행의
  기존 증거 요구를 유지한다. B07는 그 입력이나 측정 결과를 제공하지 않는다.

## 완료 조건 요약

B07.1–B07.3 구현, owner regressions, 기록된 actual stress 결과가 있어야 B07를
완료 처리한다. CI를 실행하지 않았다면 CI 상태는 open으로 별도 남긴다. 구현 중
실제 engine/host 결함이 재현되면 source-backed defect와 regression을 분리해서
수정하며, 코드 근거 없이 엔진 정책 변경이나 추가 pool/queue를 범위에 넣지 않는다.
