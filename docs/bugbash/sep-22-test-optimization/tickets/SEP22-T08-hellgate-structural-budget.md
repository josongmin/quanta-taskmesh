# SEP22-T08 Hellgate structural budget

- 상태: IMPLEMENTED_UNQUALIFIED
- finding: TO-08
- priority: P1
- write lane: `bench-validity`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh-bench/tests/hellgate.rs`

## 목적

구조 검증용 USL contention test의 고정 workload를 실험으로 최소화한다. 의미 oracle과 결정성을
유지하면서 기본 test binary 비용을 낮추되, 성능 회귀 threshold를 이 테스트에 추가하지 않는다.

## RCA

2026-09-22 baseline은 concurrency 1/2/4 각각에서 worker당 50,000 admit/release cycle을
실행해 총 350,000 cycle을 소비했다. 현재 소스는 10,000으로 줄였지만 동일 조건의 비용 절감과
cross-host fit 안정성은 아직 측정되지 않았다. 작은 workload의 measurement noise 가능성을
구조 oracle의 로컬 통과만으로 제거할 수는 없다.

## 확정 근거

- baseline 테스트는 `[1, 2, 4]` 각 점에 `contention_throughput(t, 50_000)`을 호출했다.
  현재는 `STRUCTURAL_CONTENTION_OPS_PER_THREAD = 10_000`을 사용한다:
  `crates/taskmesh-bench/tests/hellgate.rs:140-154`.
- load generator는 요청 op를 worker에 정확히 분배하고 barrier 뒤 측정하며 completed-op conservation을
  assert한다: `crates/taskmesh-bench/src/loadgen.rs:375-441`.
- throughput helper는 worker당 op count를 총 op로 변환한다:
  `crates/taskmesh-bench/src/loadgen.rs:444-451`.

## 목표 불변식

- concurrency point `[1, 2, 4]`와 세 점 USL fit 구조를 유지한다.
- 각 point의 throughput은 finite positive이고 fit/empirical peak가 복구된다.
- fixed source와 fixed workload에서 pass/fail이 반복 가능하다.
- workload는 source에 고정된 reviewable 상수이며 host/env에 따라 adaptive하게 변하지 않는다.
- performance regression authority는 Criterion/전용 benchmark receipt에 남고 unit test에 threshold를
  추가하지 않는다.

## 구현 플랜

1. baseline HEAD에서 isolated target directory를 사용해 현행 50,000 case를 macOS와 hosted Linux
   class에서 각각 10회 이상 측정한다. median, p95/worst, pass count를 기록한다.
2. candidate `20_000`, `10_000`, `5_000`, `2_000` ops-per-thread를 작은 것부터가 아니라 descending
   방식으로 시험해 cliff를 찾는다.
3. 각 candidate에서 `[1,2,4]` throughput raw values, elapsed resolution, fit 성공, empirical peak,
   20회 연속 pass를 기록한다.
4. 두 host class 모두에서 안정적인 최소 candidate보다 한 단계 높은 값을 safety margin으로 선택한다.
5. 선택값을 이름 있는 test-local constant로 두고 주석에 구조 검증 목적과 benchmark authority 분리를
   명시한다.
6. baseline 대비 median 50% 이상 절감되지 않으면 code change를 하지 않고 no-change evidence로
   finding을 닫거나 재평가한다.
7. focused case 뒤 전체 `taskmesh-bench` test binary와 gate/matrix에서 결과를 확인한다.

## 테스트와 intentional negative

- positive: 선택 workload에서 모든 point가 positive이고 fit과 peak가 복구된다.
- determinism rail: 각 host class 20회 연속 실행에서 failure 0개여야 한다.
- negative: ops-per-thread 0은 helper precondition에 의해 실패한다.
- negative: concurrency point를 2개로 줄이면 3-point structural oracle이 통과하면 안 된다.
- timing negative: source SHA, target policy, host class가 다른 sample은 비교 집합에서 제외한다.

## DoD

- SEP22-T08-A01 baseline/candidate raw 측정표에 host, source SHA, command, run count가 있다.
- SEP22-T08-A02 선택값은 두 host class에서 각 20/20 pass하고 모든 기존 구조 assertion을 유지한다.
- SEP22-T08-A03 focused median이 baseline 대비 50% 이상 감소하거나 no-change 결정이 명시된다.
- SEP22-T08-A04 unit structural test와 performance benchmark authority의 역할이 문서/주석에서 분리된다.

## 금지되는 임시방편

- CI/local 또는 CPU count에 따라 workload를 바꾸는 environment-dependent 분기.
- throughput 숫자 threshold를 unit test에 넣어 noisy performance gate로 만드는 것.
- concurrency point, fit assertion, peak assertion을 제거하는 것.
- 단 한 번의 빠른 local run만으로 workload를 선택하는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh-bench --test hellgate usl_contention_sweep_is_structurally_sound -- --exact --nocapture
cargo test --locked -p taskmesh-bench --test hellgate -- --nocapture
cargo test --locked -p taskmesh-bench
just gate
just matrix
```

성능 측정은 동일 rustc/profile/target-dir 조건에서 수행하고 concurrent Cargo/CPU workload가 있던
sample은 폐기한다.

## Stop/reopen 조건

- 작은 workload가 한 host class에서라도 fit/peak flake를 보이면 더 낮추지 않고 마지막 안정값으로
  돌아간다.
- 50% 절감과 20/20 stability를 동시에 만족하는 값이 없으면 test를 그대로 두고 Criterion/fixture
  구조 개선 티켓으로 재개한다.
- load generator 또는 production governor 변경이 필요해지면 이 소유 범위를 벗어나므로 중단한다.

## 2026-09-23 현재 소스 재검증

- `main@146233665942d75b73e2b724f781be7e105fd7c4`에 reviewable
  `STRUCTURAL_CONTENTION_OPS_PER_THREAD = 10_000`이 이미 있다. `[1,2,4]`, finite positive
  throughput, recoverable three-point USL fit, empirical peak oracle은 유지된다.
- `cargo test --locked -p taskmesh-bench --test hellgate`: exit 0, 6/6 실행·통과,
  실패/filtered/ignored 0. 현재 10k focused case는 동일한 로컬 소스에서 20/20 반복 통과했다.
- 50k baseline과 10k candidate의 동일 조건 측정 및 hosted Linux 20/20이 없다.
  호스트에서 여러 Cargo 작업이 동시에 실행되어 이번 duration은 성능 비교에 사용할 수 없다.
  50% 절감이나 두 host class 안정성은 아직 주장하지 않는다.
