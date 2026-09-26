# B07 — 필요한 recovery / soak 안정성 검증

- 상태: **CLOSED — owner-local 구현·회귀·actual stress 완료; clean CI / performance qualification 별도**
- 기준 소스: `61d4e47b0dc6009bb511704292e2a830c29c4287` + owner working delta (2026-09-27)
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
- opt-in stress는 **10분**을 시작 구성으로 사용하되 duration/cycle 수, 고정 offer trace의 manifest digest, cadence,
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
- `crates/taskmesh-bench/src/host_stability.rs` 및 `src/lib.rs`:
  typed cycle manifest/result와 순서·population·checkpoint/canary 검증만 추가한다.
  cycle record는 기존 `RawHostRecord`/`CallerCounts` 등을 재사용하고 timeline,
  offer 일치 및 count 검사 함수는 `host_load.rs`에서 공유한다. `RawHostRun`의
  terminal `drain_ok` 의미와 기존 schema는 유지한다. 중간 checkpoint를
  `drain_ok=true`인 단일 실행 raw로 위장하지 않는다.
- `crates/taskmesh-bench/tests/host_stability_integration.rs`:
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

- `crates/taskmesh-bench/examples/host_stability_probe.rs`:
  같은 host loop, create-only cycle artifact, 중단·실패 시 이미 실행한 사이클과
  실패 원인을 보존한다. cycle별 감독 프로세스 재시작으로 대체하지 않는다.
- `tools/bench/host_stability.py`: manifest 확인, 기존 build/retained
  executable/owned supervisor/resource sampler 연결, 모든 expected cycle 회계와
  typed validator 실행, 진단 report 작성. 새로운 통계적 성능 admission은 없다.
- `host_build.py` / `host_run.py`: example 허용 목록과 재사용 가능한 build/custody
  경계만 필요한 만큼 연결한다. 기존 measured admission은 바꾸지 않는다.
- `tools/bench/tests/test_host_stability.py`: 빠진/중복/재정렬 cycle,
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
owner-local PASS와 clean CI, 실제 장시간 실행 결과는 각각 기록한다. 현재 구현은 완료되었고 최종 owner suite와 실제 stress 결과는 아래 실행 기록으로
확정했다. 중단되거나 실행하지 않은 상태를 PASS로 취급하지 않는다.

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


## 구현 경계와 실행 기록 — 2026-09-27

- `StabilityManifest` v1: default feature만 허용하는 Python acquisition, IO/blocking/CPU,
  cycles 1..10,000, duration ≤20분, total records ≤1,000,000,
  전체 cycle snapshots + checkpoints + summary endpoints ≤1,000,000. 자원 sampler는 기존 20,000 sample cap.
- `StabilityCycle` v1의 nested raw는 `drain_ok=false`; 기존 단일 실행 raw validator는
  이를 거부한다. 내부 window validator만 명시적으로 nonterminal 상태를 검증한다.
- cycle마다 class cumulative counters의 정확한 delta, 동일 capability/substrate inventory,
  zero-owned checkpoint, 각 사용 class/path의 실제 body 실행과 Success canary를 검증한다.
- complete-population CLI는 0..N-1 파일을 순서대로 읽고 baseline→최종 snapshot을 연결한다.
  cycle 하나만 저장하거나 성공 cycle만 선택하면 PASS가 불가능하다.
- cycle은 기존 create-only atomic artifact writer로 flush; runner는 현재 cycle과 이전
  snapshot만 유지한다. Python replay는 각 regular descriptor의 digest를 검증해 private
  bundle을 만든 다음, 그 안의 실행 파일과 입력만 120초 감독 하에 실행한다.
- 기존 build/witness selector, owned subprocess supervision, same-PID sampler를 재사용한다.
  source/host identity는 build 전후·launch 직전·실행 종료를 비교한다. 이는 endpoint
  관측이며 악의적인 변경 후 원복이나 독립 cold rebuild를 증명하지 않는다.
- 실패/timeout/interrupt는 이미 flush된 cycle, stderr, resources와 invalid receipt를 보존한다.
  임의 SIGKILL, 디스크 오류로 기록 자체가 불가능한 경우에는 완전한 실패 artifact를 보장하지 않는다.
- smoke fixture 3개(원래 H2/H5 trace를 내장한 3-cycle H2/H5, 1,000-cycle H2 stress)가
  원래 workload와 일치하는 regression을 둔다. RNG를 사용하지 않으므로 가짜 seed 필드는 없다.
- 실제 10분 stress는 **H2 1,000회 반복 burst + cycle 사이 idle**이다. 지속 saturation,
  production 장기 soak, RSS leak 부재 또는 latency/recovery SLO 증거로 승격하지 않는다.
- 최종 owner suite / 실제 stress: **PASS — 아래 frozen-source 증거 참조**.
  clean CI, mutation, release, push: 미실행.


### 최종 boundary 감사와 중단 증거

- 실제 H5 bundle을 복사해 receipt `schema_version`을 JSON boolean `true`로 바꿨을 때,
  Python의 `True == 1` 비교 때문에 기존 검증이 이를 승인했다 (**behavioral RED**).
  receipt version을 exact integer로 제한했고 boolean/float/string/0/2 거부 회귀를 추가했다.
  Rust manifest/cycle/summary는 typed serde로 동일한 scalar 혼동을 이미 거부한다.
- 첫 H2 stress는 이 추가 감사를 반영하기 위해 controller PID에 실제 SIGINT를 전달해 중단했다.
  **425 cycle**이 남았고 runner PID **73864**, exit **-1**, receipt **invalid**,
  resources **unavailable**, stderr `benchmark interrupted by signal 2`가 보존됐다.
  해당 owner process/runner가 종료됐음을 확인했다. 이 실행은 successful stress에서 제외한다.
  Bundle: `/tmp/taskmesh-b07-h2-stress-61d4e47/`; log: `/tmp/taskmesh-b07-stress.log`.
- 수정 후 새 디렉터리에서 전체 1,000 cycle / 600초 stress를 다시 시작한다.
  중단 cycle과 새 study를 이어 붙여 1,000회 완료라고 계산하지 않는다.


### Construction 감사 보완

- `HostScenario::resolved_topology()`는 실제 `build_runtime()`을 호출한다. 첫 구현의
  online raw/cycle validation이 이를 반복 호출한 것이 source audit에서 확인됐다.
  current-thread factory counter regression은 3 cycle에 **7회 생성 vs 기대 1회**로
  실패했다. 0-case selector 시도는 증거로 세지 않았고 정확한 test 경로의 1-case
  behavioral RED만 사용했다 (`/tmp/taskmesh-b07-construction-red-exact.log`).
- online validation은 최초 host의 installed topology를 고정해 shared raw validator에
  전달한다. 별도 replay process는 manifest에서 oracle topology를 1회만 구성한다.
  `run` CLI는 online 검증 뒤 파일을 기록하며, 별도 Python supervisor가 실행 종료 후
  전체 artifact replay를 수행한다. 측정 PID에서 replay용 host를 만들지 않는다.
- factory regression이 실제 **1회 생성**으로 통과했다. 누적 counters만으로 검증에
  사용한 보조 host 생성까지 감지할 수 있다고 주장했던 부분을 이 회귀로 보완한다.
- global snapshot cap에는 cycle당 3 checkpoints와 summary의 baseline/final 2개도
  포함한다. cap 경계의 manifest 거부/바로 아래 허용 회귀를 추가했다.
- 두 번째 H2 stress는 이 construction 수정 전 실제 SIGINT로 중단했다. **262 cycle**,
  invalid receipt와 중단 stderr를 보존했다. Bundle:
  `/tmp/taskmesh-b07-h2-stress-final-61d4e47/`. 성공 실행에 포함하지 않는다.
- 변경 파일에 `crates/taskmesh-bench/src/host_scenarios.rs`의 test-only thread-local
  factory counter가 추가됐다. product Taskmesh runtime/API/policy에는 변경이 없다.


## Owner-local closure — frozen working source

**B07.1 / B07.2 / B07.3: 완료.** 아래는 같은 소스에서 실행한 owner checks와 실제
진단이다. 다른 소유자의 dirty 변경을 보존했으므로 clean/exact-commit CI 또는
release qualification으로 사용하지 않는다. 코드 검증 뒤 이 문서의 완료 기록을
수정했으며, receipt의 source identity는 그 수정 이전 실행 snapshot을 가리킨다.

- 기준 HEAD: `61d4e47b0dc6009bb511704292e2a830c29c4287` + B07 working delta.
- 실행 중 source/host endpoint 일치 및 `verify --require-current-source`: PASS.
- 두 완료 study의 source content digest:
  `d4ed183a7900404ace0a4d299018df379f365f2173910a2f89a76c4d206c52f3`.
- 두 study의 실제 retained default-feature debug runner digest:
  `edc24aaed61ff902d53c0bd2b93aca07f420f8dd5eb5baa5ea10041c0cebf70f`.
- artifact는 byte/digest 검증 후 ignored `bench-results/receipts/owner-b07-61d4e47/`에
  복사하고 다시 검증했다. complete와 interrupted study는 별도 디렉터리에 보존한다.
  Git-tracked 또는 외부 재실행 가능한 qualification receipt는 아니다.

### 완료된 검증

| 검증 / 정확한 명령 | 결과 / claim |
|---|---|
| `cargo clippy --locked -p taskmesh-bench --all-targets -- -D warnings` | exit 0; Justfile `clippy`의 bench 전용 owner 정책 |
| `just dev-rust-tests taskmesh-bench` | exit 0; **118 PASS / 0 FAIL**, 14 test targets. 실제 factory 1회, held-worker checkpoint 거부, H2/H5 반복, IO/blocking/CPU canary 및 기존 raw 회귀 포함 |
| `CARGO_BUILD_JOBS=4 cargo test --locked -p taskmesh-bench --features rayon --test host_load_integration --test host_stability_integration -- --test-threads 4` | exit 0; **13 PASS / 0 FAIL**. optional feature의 factoring 회귀이며 Rayon stress/performance admission은 아님 |
| `uv run pytest -q tools/bench/tests tools/gates/tests/test_batch_supervisor.py tools/qualification/tests/test_evidence.py tools/qualification/tests/test_receipt.py -m 'not slow and not qualification' --tb=short` | exit 0; **616 PASS / 0 FAIL / 71 deselected**, 330.48초. excluded cases는 실행 증거로 세지 않음 |
| `just dev-python-tests tools/bench/tests/test_host_stability.py` | **32 PASS**. 실제 diagnostic runner/replay 사용; identity/build 일부는 unit orchestration fixture로 고정하므로 실제 host provenance는 아래 study가 별도 제공 |
| `just dev-python-lint tools/bench/host_stability.py tools/bench/tests/test_host_stability.py tools/bench/host_run.py tools/bench/host_build.py` | exit 0 |
| `cargo fmt --package taskmesh-bench --check` / `git diff --check` | exit 0 |
| `uv run python tools/gates/validate_inventory.py` | exit 0; 24 gates, 23 required, 181 discovered targets. discovery는 실행 증거가 아님 |

Logs: `bench-results/receipts/owner-b07-61d4e47/taskmesh-b07-{final-clippy-frozen,final-rust-frozen,final-python-frozen,rayon-frozen}.log`.

초기 `dev-rust-fast`는 production용 pedantic/restrict lint를 bench에도 적용해 실패했다.
저장소 `Justfile::clippy`는 bench에 `--all-targets -- -D warnings`를 적용하므로 이
기존 owner 정책으로 확인했다. baseline 또는 타 owner 파일을 고치거나 규칙을 완화하지 않았다.
중간 Python 실행은 **614 PASS / 2 FAIL**이었다: 0.4초 timeout 전에 child PID fixture가
생성되지 않은 setup failure, 소스 편집 중 minimal control이 source drift를 정상 거부한
실패다. cleanup 4-case 진단이 별도로 PASS했고, 편집을 멈춘 전체 재실행의 **616 PASS**가
최종 증거다. 실패 실행을 green으로 바꾸거나 subset 합격으로 대체하지 않았다.

### 실제 완료 study

```sh
uv run python tools/bench/host_stability.py run \
  tools/bench/scenarios/h2-stability-stress.json \
  /tmp/taskmesh-b07-h2-stress-one-host-61d4e47 --cadence-ms 250 --timeout-seconds 900
uv run python tools/bench/host_stability.py verify \
  /tmp/taskmesh-b07-h2-stress-one-host-61d4e47 --require-current-source
uv run python tools/bench/host_stability.py run \
  tools/bench/scenarios/h5-stability-smoke.json \
  /tmp/taskmesh-b07-h5-smoke-one-host-61d4e47 --cadence-ms 25 --timeout-seconds 120
uv run python tools/bench/host_stability.py verify \
  /tmp/taskmesh-b07-h5-smoke-one-host-61d4e47 --require-current-source
```

| Study | 실제 결과 | 보존 bundle / receipt SHA-256 |
|---|---|---|
| H2 opt-in stress | exit 0; PID **75663**; **600.002572초 / 1,000 cycle**; 15,000 window rows + 1,000 real canaries; 10,000 window snapshots; host 생성 1회; 각 cycle exact zero checkpoint·admission open; 최종 drain/closed admission PASS; resources complete, **2,361 samples** | `bench-results/receipts/owner-b07-61d4e47/h2-stress/`; `61ba95d810936ed700af67608a749a81b2c1becfd31c7d339bcbb3e2827709ad` |
| H5 smoke | exit 0; PID **89186**; **3 cycle**; 18 window rows + 3 real canaries; 30 window snapshots; host 생성 1회; 최종 drain PASS; resources complete, **13 samples** | `bench-results/receipts/owner-b07-61d4e47/h5-smoke/`; `f046c529c4532b1d9f2fb2090902ec80358dba6106d97a306302e97abe689965` |

두 결과는 **functional PASS / performance UNQUALIFIED**다. sampler `complete`는
관측이 가능했음을 뜻하며 leak 부재·정확한 high-water·CPU-per-success 또는
일정 sample 간격을 보장하지 않는다. 10분 H2는 반복 burst 사이 idle을 포함한다.

### 남은 범위

- **B07 owner-local 코드 / 회귀 / 실제 diagnostic stress:** 이 티켓에서 남은 항목 없음.
- **Clean-source CI 및 optimized frozen build/oracle E2E:** 미실행 / 별도 open.
- **B00 estimand·consumer budget, H7 profile, quiet-host series, peer 비교:** 기존 입력/측정
  공백 유지. B07 실행이 이를 채우지 않는다.
- **Conditional all-mode performance admission / RSS slope / recovery SLO:** 보류 유지.
- mutation, nightly/release, 배포, remote push: 이번 owner-local 범위에 포함하지 않았다.
