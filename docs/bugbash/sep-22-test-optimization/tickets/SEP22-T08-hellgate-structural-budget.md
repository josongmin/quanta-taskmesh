# SEP22-T08 Hellgate structural budget

- 상태: IMPLEMENTED_UNQUALIFIED — 2,000-cycle owner-local 후보 선택; clean-source gate/matrix 미실행
- finding: TO-08
- priority: P1
- write lane: `bench-validity`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh-bench/tests/hellgate.rs`

## 목적

구조 검증용 USL contention test의 고정 workload를 줄인다. admit/release 보존과 세 동시성
점의 유한한 측정값을 유지하고, 줄인 workload에서의 반복 통과와 비용 감소를 두 host class에서
확인한다. 성능 회귀 threshold는 이 테스트에 추가하지 않는다.

## RCA

2026-09-22 baseline은 concurrency 1/2/4 각각에서 worker당 50,000 admit/release cycle을
실행해 총 350,000 cycle을 소비했다. 10,000으로 낮춘 뒤에도 구조 테스트 한 번에 70,000
cycle을 실행했다. 이 테스트는 throughput threshold가 없고, 세 양의 측정값으로 USL fit이
되는지 확인할 뿐이다. 반복 통과와 두 host class 비교가 USL 성능 정확도를 증명하지는
않지만, timing에 의존하는 구조 oracle을 줄인 workload에서 안정적으로 실행할 수 있는지는
최초 ticket의 별도 acceptance다.

## 확정 근거

- baseline 테스트는 `[1, 2, 4]` 각 점에 `contention_throughput(t, 50_000)`을 호출했다.
  현재는 `STRUCTURAL_CONTENTION_OPS_PER_THREAD = 2_000`을 사용한다:
  `crates/taskmesh-bench/tests/hellgate.rs:140-154`.
- load generator는 요청 op를 worker에 정확히 분배하고 barrier 뒤 측정하며 completed-op conservation을
  assert한다: `crates/taskmesh-bench/src/loadgen.rs:375-441`.
- throughput helper는 worker당 op count를 총 op로 변환한다:
  `crates/taskmesh-bench/src/loadgen.rs:444-451`.

## 목표 불변식

- concurrency point `[1, 2, 4]`와 세 점 USL fit 구조를 유지한다.
- 각 point의 throughput은 finite positive이고 fit/empirical peak가 복구된다.
- fixed source와 fixed workload에서 pass/fail이 두 host class에서 반복 가능하다.
- workload는 source에 고정된 reviewable 상수이며 host/env에 따라 adaptive하게 변하지 않는다.
- performance regression authority는 Criterion/전용 benchmark receipt에 남고 unit test에 threshold를
  추가하지 않는다.

## 구현 플랜

1. 현재 2,000 ops-per-thread는 test-local constant다. 세 점의 합계는 14,000
   admit/release cycle이며, 각 cycle의 성공·반납과 총 완료 수 보존은 load generator가 검사한다.
2. 동일 rustc/profile/target policy의 macOS와 Linux에서 50,000-cycle baseline 및
   20,000/10,000/5,000/2,000/1,000-cycle 후보를 큰 값부터 시험한다. 각 host의 source SHA,
   command, pass count, raw duration, median, worst, 세 throughput, fit/peak를 남긴다.
3. 두 host class에서 안정적인 최소 후보보다 한 단계 높은 값을 safety margin으로 고른다.
   선택 후보 exact case는 host별 20/20 통과해야 하고, focused median은 같은 host의
   50,000-cycle baseline보다 최소 50% 낮아야 한다. 조건을 못 채우면 no-change로 닫거나
   workload/fixture 설계를 재평가한다.
4. 최종 후보에서 전체 bench test, gate, matrix를 동일 clean source로 실행한다. 작업량
   감소를 throughput 성능 회귀 방어라고 주장하지 않는다.

## 테스트와 intentional negative

- positive: 선택 workload에서 모든 point가 positive이고 fit과 peak가 복구된다.
- determinism rail: macOS와 Linux 각각 후보 exact case 20회에서 failure 0개여야 한다.
- negative: ops-per-thread 0은 helper precondition에 의해 실패한다.
- negative: concurrency point를 2개로 줄이면 3-point structural oracle이 통과하면 안 된다.
- timing negative: source SHA, profile, target policy, host class가 다른 sample을 같은
  baseline/candidate 비교 집합으로 합치지 않는다.

## DoD

- SEP22-T08-A01 macOS와 Linux baseline/candidate raw 측정표에 host, source SHA, command,
  profile, target policy, run count, median, worst가 있으며 candidate ladder와 safety-margin
  선택 근거가 기록된다.
- SEP22-T08-A02 선택값은 두 host class에서 각각 20/20 pass하고 기존 구조 assertion을 유지한다.
- SEP22-T08-A03 host별 focused median이 baseline 대비 50% 이상 감소하거나 no-change 결정이 명시된다.
- SEP22-T08-A04 unit structural test와 performance benchmark authority의 역할이 문서/주석에서 분리된다.

## 금지되는 임시방편

- CI/local 또는 CPU count에 따라 workload를 바꾸는 environment-dependent 분기.
- throughput 숫자 threshold를 unit test에 넣어 noisy performance gate로 만드는 것.
- concurrency point, fit assertion, peak assertion을 제거하는 것.
- 구조 테스트 통과 횟수를 throughput 성능 보증으로 해석하는 것.
- macOS focused 통과만으로 Linux 반복 안정성이나 원래 acceptance를 닫는 것.

## 검증 명령

```sh
cargo test --locked -p taskmesh-bench --test hellgate usl_contention_sweep_is_structurally_sound -- --exact --nocapture
cargo test --locked -p taskmesh-bench --test hellgate -- --nocapture
cargo test --locked -p taskmesh-bench
just gate
just matrix
```

비용 비교는 동일 rustc/profile/target-dir 조건에서 수행하고 concurrent Cargo/CPU
workload가 있던 sample은 폐기한다.

## Stop/reopen 조건

- 선택된 2,000-cycle 구조 테스트가 실패하면 수치만 키워 통과시키지 않고 실패 원인을 확인한다.
- 어느 host에서든 20/20 안정성이나 50% 비용 감소를 만족하지 못하면 더 낮은 workload를
  정당화하지 않고 이전 안정값 또는 no-change 결정을 검토한다.
- 성능 회귀 검출이 필요하면 이 smoke에 noisy threshold를 추가하지 않고 Criterion/IAI owner에서
  별도 처리한다.
- load generator 또는 production governor 변경이 필요해지면 이 소유 범위를 벗어나므로 중단한다.

## 2026-09-23 이전 10,000-cycle 소스 재검증

- `main@146233665942d75b73e2b724f781be7e105fd7c4`에 reviewable
  `STRUCTURAL_CONTENTION_OPS_PER_THREAD = 10_000`이 이미 있다. `[1,2,4]`, finite positive
  throughput, recoverable three-point USL fit, empirical peak oracle은 유지된다.
- `cargo test --locked -p taskmesh-bench --test hellgate`: exit 0, 6/6 실행·통과,
  실패/filtered/ignored 0. 현재 10k focused case는 동일한 로컬 소스에서 20/20 반복 통과했다.
- 50k baseline과 10k candidate의 동일 조건 측정 및 hosted Linux 20/20이 없다.
  호스트에서 여러 Cargo 작업이 동시에 실행되어 이번 duration은 성능 비교에 사용할 수 없다.
  50% 절감이나 두 host class 안정성은 아직 주장하지 않는다.

## 2026-09-23 이전 10,000-cycle 구조 오라클 보강

- `[1,2,4]`의 USL normal-equation determinant는 고정값 36이다. 양의 throughput 세 점에서
  `fit_usl(...).is_some()`은 timing noise를 실질적으로 판별하지 못하고, 기존 `fit.x1 > 0`
  검사만으로는 비유한 `alpha`/`beta`도 통과할 수 있었다.
- 현재 테스트는 baseline과 두 계수의 유한성, 반환된 peak가 실제 측정 샘플이며 그중 최대
  throughput인지 확인한다. concurrency 세 점과 workload 10,000은 그대로 유지했다.
- focused case 20/20, 전체 `hellgate` 6/6, 해당 test-target Clippy, rustfmt check가 통과했다.
  이 반복은 기능 안정성 증거이지, 부하가 큰 호스트의 duration을 50k 대비 절감률로 해석한
  증거가 아니다. 동일 조건의 50k/10k 비용 비교와 Linux 실행은 여전히 열려 있다.

## 2026-09-23 최소 구조 smoke 조정

- 당시 소스는 worker당 1,000 cycle, 세 점 합계 7,000 cycle을 요청했다. 이는 10,000-cycle
  소스보다 고정 작업량이 90% 적다. 테스트는 성능 threshold를 검사하지 않으므로 이 차이를
  wall time 절감률이나 성능 안정성으로 해석하지 않는다.
- `main@1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b` 위 T08 owner 변경으로
  `cargo test --locked -p taskmesh-bench --test hellgate -- --nocapture`가 6/6 통과했다.
  `cargo fmt --all -- --check`, ticket structure validator, `git diff --check`도 통과했다.
  호스트 부하가 높았으므로 19.93초 test-binary duration을 비용 비교에 쓰지 않는다.
- 당시 문서 수정은 candidate ladder·safety margin·20/20·Linux 비교를 acceptance에서
  제거했다. 이 시점 감사에서는 원래 비용·반복 안정성 목표를 다시 완료 조건으로 뒀다.
  당시 macOS 증거는 부분 충족이었고 ladder, Linux 비교와 clean-source gate/matrix가
  없었다. 1,000-cycle 값은 최종 선택으로 인정하지 않았다.

## 2026-09-23 동일 소스 focused 비용 대조

- `main@1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b`의 동일 production/구조
  assertion에서 worker당 op 상수만 1,000과 50,000으로 바꾼 두 exact test binary를
  `cargo test --locked -p taskmesh-bench --test hellgate --no-run`의 `test` 프로필,
  같은 target directory로 빌드했다. 후보 owner SHA-256은
  `7cb7241da0057a69e84c3cb06f851f1b635ee563842ac990905a986bd5d01125`,
  50,000-cycle 대조 소스 SHA-256은
  `5a48eb9a7eaf300c1de43a2ef7f12e842a6e2d146e2835f0dca95ecdcd7b2399`이다.
- macOS 15.6 arm64에서 `usl_contention_sweep_is_structurally_sound --exact`를
  교차 순서로 각 10회 실행해 두 변형 모두 10/10 통과했다. Process wall median/worst는
  1,000-cycle 후보 0.0891/0.7376초, 50,000-cycle 대조 3.9653/4.4518초였다.
  중앙값 차이는 이 호스트의 focused 실행에서 97.75%였다. 후보 binary를 추가 10회
  실행해 누적 20/20 통과했다. 임시 worktree의 source는 원상 복원됐다.
- 이 비교는 고정 작업량 감소의 로컬 비용 효과를 확인한다. 동시 호스트 부하가 있어
  worst outlier가 있고 Linux·전체 gate·성능 회귀 결과로 확대하지 않는다. W3
  exact-source full receipt는 여전히 필요하다.

## 2026-09-23 macOS 후보 ladder — 부분 증거

- [원시 측정값](../T08-MACOS-LADDER-2026-09-23.json)은 동일 production/source assertions에서
  op 상수만 바꾼 6개 binary의 source/binary SHA-256, compile command, host/rustc,
  교차 순서의 10회 process wall sample을 보관한다. 임시 worktree와 target은 정리했고
  당시 1,000-cycle owner SHA-256은 위 기록과 같다.
- 각 변형의 exact test는 10/10 통과했다. 중앙값/최악값(초)은 50k `3.283/3.943`,
  20k `1.352/1.748`, 10k `0.626/0.824`, 5k `0.304/0.567`, 2k `0.120/0.359`,
  1k `0.066/0.304`였다. 50k 대비 이 실행의 중앙값 감소율은 20k부터 각각
  `58.83%`, `80.93%`, `90.75%`, `96.36%`, `98.00%`다.
- macOS에서 관측한 최소 통과 후보는 1k이고 한 단계 safety margin 후보는 2k다.
  다른 작업의 실행을 전 기간 감시하지 않았고 세 throughput 원시값도 출력하지 않았으므로
  quiet-host 비용 증거나 A01 완료로 올리지 않는다. Linux ladder·20/20·원시 throughput,
  최종 후보 선택과 clean-source gate/matrix가 당시에는 남았다. 현재 소스는 아래
  동일 toolchain의 두 환경 ladder 후 2k로 변경했다.

## 2026-09-23 동일 Rust 1.92.0 두 환경 후보 선택

- 두 별도 detached checkout은 `main@1d2bb2f`의 동일 production source와 동일 test profile을
  사용했다. `hellgate.rs`의 고정 op 상수만 50k/20k/10k/5k/2k/1k로 바꾸어 각 binary를
  빌드했다. 내부 계측 뒤 세 throughput 값을 출력하도록 한 현재 테스트 코드가 모든 변형에
  공통이다. 선택된 2k 변형의 SHA-256 `aeaaa0abea0f555355d7d21fec5e0d4f236864421fc6c49df1d31c63f0055ce8`은
  현재 owner 파일과 일치한다.
- [macOS 원시 실행·throughput](../T08-MACOS-RUST192-LADDER-2026-09-23.json)과
  [Linux 원시 실행·throughput](../T08-LINUX-CONTAINER-LADDER-2026-09-23.json)은
  교차 순서 10회/후보, 2k 추가 10회, 각 variant의 source/binary SHA-256, command, 세
  throughput 및 process wall sample을 보관한다. Linux는 같은 Mac 하드웨어 위 Docker
  Linux VM(`rust:1.92-bookworm`, network off)이다. OS 동작은 Linux지만 독립 물리
  호스트의 성능 대표성까지 주장하지 않는다.

| 환경 | 50k median/worst (s) | 1k 통과 | 선택 2k 통과 | 2k median/worst (s) | 50k 대비 2k median 감소 |
| --- | ---: | ---: | ---: | ---: | ---: |
| macOS arm64 | 3.489615 / 5.715192 | 10/10 | 20/20 | 0.134249 / 0.358407 | 96.15% |
| Docker Linux arm64 | 6.043358 / 6.126708 | 10/10 | 20/20 | 0.252677 / 0.269119 | 95.82% |

- 두 환경의 최소 관측 통과값은 1k다. 한 단계 높은 2k를 safety margin으로 선택했다.
  세 throughput은 모든 실행에서 finite positive였고 fit/peak assertion도 통과했다.
  macOS 20k 실행에는 12.55초 outlier가 있었다. 호스트 작업을 전 기간 감시하지
  않았으므로 quiet-host 성능 비교나 성능 회귀 방어 결과로 승격하지 않는다.
- 선택 source의 `cargo test --offline --locked -p taskmesh-bench`는 Rust 1.92.0에서
  macOS와 Docker Linux 각각 55 unit + 1 fairness + 6 hellgate + 10 inferno = 72/72
  통과했다. 기존 `metrics::tests::usl_needs_enough_points`는 두 점 fit을 거절하며
  `contention_throughput`은 0 ops를 직접 거절한다. 이 결과는 owner-local이다.
  clean-source 전체 `just gate`, `just matrix`, `just verify-macos-full`과 Linux
  ordinary qualification은 별도로 남는다.
