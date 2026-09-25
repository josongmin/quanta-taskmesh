# SEP22-T08 Hellgate structural budget

- 상태: LOCALLY_VERIFIED — 중복 real-thread USL smoke 제거; clean-source gate는 별개
- finding: TO-08
- priority: P1
- write lane: `bench-validity`
- 선행 티켓: 없음
- 소유 경로: `crates/taskmesh-bench/tests/hellgate.rs`

## 목적

Hellgate에서 독립적인 실패 판정 기준이 없는 contention 반복을 제거한다. 완료 수 보존,
throughput 계산, USL fit/peak의 수학적 oracle은 각각 원래 owner test에 남긴다.

## RCA

기존 `usl_contention_sweep_is_structurally_sound`는 concurrency `[1, 2, 4]`마다
worker당 50,000회에서 시작해 이후 2,000회까지 admit/release를 반복했다. 각 점의
throughput이 finite positive인지, 세 점으로 USL fit이 가능한지, `argmax_throughput`이
입력 중 최대값을 되돌리는지만 검사했다. 성능 기준이나 기대되는 α/β 값은 없었다.
반복 횟수를 줄이는 방식은 비용을 줄일 뿐 새 oracle을 만들지 않는다.

## 확정 근거

- `contention_run`은 worker별 허가·반납과 전체 완료 수를 직접 검사한다:
  `crates/taskmesh-bench/src/loadgen.rs:373-441`.
- owner unit test는 1/2/4/8 worker의 정확한 완료 수, known-duration throughput 산술,
  최소 real-thread helper의 finite positive 결과를 검사한다:
  `crates/taskmesh-bench/src/loadgen.rs:738-774`.
- metrics unit test는 알려진 USL 계수의 복원과 empirical peak를 독립적인 기대값으로
  검사한다: `crates/taskmesh-bench/src/metrics.rs:170-187,206-223`.
- Criterion contention benchmark는 1/2/4/8 worker의 정확한 denominator로 실제
  측정 경로를 유지한다: `crates/taskmesh-bench/benches/contention.rs:10-33`.
- 이전 Hellgate smoke가 유일하게 추가하던 것은 같은 실행에서 loadgen 출력과 USL fit을
  연결하는 통합 확인이었다. 그러나 세 finite positive 점의 fit은 이 고정 x축에서 거의
  항등적으로 가능하고, peak 검사는 `argmax_throughput` 결과를 다시 입력에 대조하므로
  버그 판별력이 낮다. 이 통합 smoke를 제거하는 coverage 손실은 명시적으로 수용한다.

## 목표 불변식

- `hellgate`의 나머지 다중 seed/부하 구간, fail-closed, tail, 결정성 oracle은 유지한다.
- admit/release 완료 수와 throughput helper의 finite positive oracle은 loadgen owner에
  유지한다. USL 계수와 peak의 의미 검증은 metrics owner에 유지한다.
- 성능 회귀는 Criterion/전용 benchmark receipt가 맡으며, wall-time 기반 unit test는
  만들지 않는다.

## 구현 플랜

1. `hellgate.rs`의 USL sweep, op-count 상수, 불필요한 imports와 헤더 주장을 삭제한다.
2. `loadgen.rs`, `metrics.rs`, Criterion benchmark의 실제 oracle 존재를 대조한다.
3. Hellgate의 나머지 5개 테스트는 exact substitute oracle이 없으므로 유지한다:
   다중 seed/regime conservation, 정확한 queue 포화, Poisson undersaturation,
   reject/tail monotonicity, 동일 seed 결정성은 기존 owner unit test와 입력 또는
   판정이 다르다.
4. focused bench suite와 문서 구조 validator를 실행하고 clean-source gate/qualification은
   별도 W2/W3 판정에 남긴다.

## 테스트와 intentional negative

- positive: `taskmesh-bench` 전체 test가 통과하고 `hellgate`는 5개만 실행된다.
- negative: `contention_run`의 허가 실패·반납 실패·완료 수 불일치 시 owner assertion이
  실패한다. USL 수학의 잘못된 계수/peak는 known-model metrics unit test가 검출한다.
- 삭제한 smoke의 real-thread throughput→fit 조합 자체는 더 이상 gate되지 않는다.
  이 조합에 product-level 요구사항이 생기면, 기대값 또는 명시적 failure mode를 가진
  별도 owner oracle로 재도입한다.

## DoD

- SEP22-T08-A01 중복 USL Hellgate 테스트와 op-count 상수/import를 제거한다.
- SEP22-T08-A02 loadgen/metrics의 독립 oracle과 실제 성능 benchmark 경로를 확인한다.
- SEP22-T08-A03 나머지 Hellgate 5개를 보존하고 focused bench suite와 구조 validator를
  통과시킨다.
- SEP22-T08-A04 제거로 잃는 단일 통합 smoke coverage와 clean-source qualification
  미실행 상태를 명시한다.

## 금지되는 임시방편

- 반복 횟수만 바꿔 독립 검증이 생긴 것처럼 해석하지 않는다.
- `fit_usl(...).is_some()`만으로 성능/스케일링이 정상이라고 주장하지 않는다.
- 성능 threshold를 timing-noisy unit test에 넣지 않는다.
- owner tests 또는 benchmark를 제거해 coverage 손실을 숨기지 않는다.

## 검증 명령

```sh
cargo test --locked -p taskmesh-bench --test hellgate
cargo test --locked -p taskmesh-bench
cargo fmt --all -- --check
uv run python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py --structure-only
git diff --check
```

## Stop/reopen 조건

- 삭제 후 loadgen/metrics의 owner oracle이 실제로 없거나 실패하면 제거를 닫지 않고
  해당 owner에서 독립적 의미 oracle을 복구한다.
- 실측 throughput→USL pipeline에 수치 정확도 요구가 생기면 synthetic known-model
  입력 또는 독립된 expected result를 정의한다. 임의의 반복 횟수/20회 재실행은 대체물이 아니다.
- clean-source 전체 gate/matrix/qualification은 이 focused 검증으로 대신하지 않는다.

## 역사적 측정 기록

기존 50k→1k/2k 후보 ladder와 macOS/Docker Linux 반복 측정은 이미 수행한 비용 비교
기록이다: [macOS ladder](../../../../bugbash/sep-22-test-optimization/T08-MACOS-RUST192-LADDER-2026-09-23.json),
[Linux ladder](../../../../bugbash/sep-22-test-optimization/T08-LINUX-CONTAINER-LADDER-2026-09-23.json). 2k 후보는 두 환경에서
각각 20/20 통과했고, 50k 대비 focused median은 각각 96.15%/95.82% 낮았다. 이것은
당시 *중복 smoke를 더 싸게 실행*한 증거일 뿐 smoke의 검증 가치나 제품 성능을
증명하지 않는다. 따라서 후보 ladder·safety margin·Linux 20/20은 현재 acceptance에서
제외한다.

## 현재 owner-local 검증

- `cargo test --locked -p taskmesh-bench --test hellgate`: 5/5 pass.
- `cargo test --locked -p taskmesh-bench`: unit 55/55, fairness 1/1, Hellgate 5/5,
  Inferno 10/10, doctest 0; 합계 71개 pass.
- `cargo fmt --all -- --check`, `git diff --check`, ticket structure validator는 pass.
- 이는 T08 owner-local 증거다. shared working tree의 다른 수정이 있었고 clean-source
  `just gate`/`just matrix`/qualification은 이 실행으로 증명하지 않는다.
