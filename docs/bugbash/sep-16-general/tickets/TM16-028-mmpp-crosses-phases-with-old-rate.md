# TM16-028 — Bursty generator가 이전 phase rate로 다음 phase를 건너뛴다

- Severity: P2
- Status: OPEN / seeded observation reproduced
- Lane: B — benchmarks
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-bench/src/workload.rs:128-140`: 현재 phase rate로 inter-arrival 전체를 샘플링해 시간을 먼저 이동한 뒤, 지나간 phase boundary만 뒤늦게 갱신한다.
- `crates/taskmesh-bench/src/workload.rs:328-351`: 기존 검증은 seed determinism/monotonicity/CV이며 CTMC rate 정확성을 검사하지 않는다.
- [Retained observation](evidence/repro-bench/src/lib.rs): `burst_generator_misses_short_high_rate_phases`.

## Trigger / 관찰

seed=42, count=1000, low=1/s, high=1000/s, off/on mean dwell 각각 1ms에서 실제 생성 rate는 **2.648265/s**다. 올바른 equal-dwell CTMC의 stationary offered rate는 `(1 + 1000)/2 = 500.5/s`다. 시작이 off phase인 transient만으로 이 차이를 설명할 수 없다.

## 원인 / 영향 / 범위

off rate의 긴 sample이 짧은 high-rate 구간들을 통째로 지나간다. 올바른 next event는 arrival와 phase transition 중 먼저 오는 사건이어야 한다. 현재 구현은 transition 동안 발생했어야 할 arrivals를 생성하지 않는다. 높은 CV와 deterministic replay를 만족해도 지정한 MMPP가 아니다. Burst load 기반 backlog/reject/fairness 검증이 실제 목표 부하보다 훨씬 약해질 수 있다.

## 보완 계획

- current-rate arrival와 phase boundary를 비교하여 boundary가 먼저면 시간을 boundary로 이동하고 rate 변경 후 새 sample을 생성한다. 또는 integrated hazard 기반 reference 구현을 사용한다.
- seed RNG 사용 순서가 바뀌므로 이전 trace와 새 generator version을 구분한다.
- determinism/CV뿐 아니라 configured CTMC의 stationary rate 및 phase occupancy를 독립 oracle와 비교한다.

## Acceptance / 회귀 검증

- 빠른 equal-dwell switching에서 장기 rate가 500.5/s의 통계적 tolerance에 들어온다. seed 42 하나를 새로운 golden number로 고정하지 않는다.
- equal low/high rates는 ordinary Poisson rate로 수렴한다.
- long/short/asymmetric dwell, 여러 seeds, phase boundary에서 event scheduling을 검증한다. 입력 rate/dwell은 finite positive로 검증한다.

## 추가 감사 (5) — zero-dwell nonprogress 재현

- `workload.rs:121-122`는 `mean_off_secs=mean_on_secs=0`에서 `Exp::new(1/0)`을 호출한다. Locked rand_distr 0.4.3 `src/exponential.rs:135-149`는 positive infinity를 허용하고 inverse=0으로 sampling한다. `expect("mean_off > 0")`가 실제 dwell 검증을 하지 못한다.
- count=1, 나머지 기본 설정에서도 phase_end가 계속 0이다. `while t >= phase_end`에서 0을 무한히 더하여 첫 arrival을 만들지 못한다.
- [Retained isolated observation](evidence/repro-bench/src/lib.rs): `zero_dwell_burst_config_never_advances_phase_boundary`. positive-dwell control은 1건을 반환한다. zero-dwell child는 진입 marker를 출력한 뒤 1초 동안 반환하지 않았으며 parent가 kill/wait했다. 무한 진행 실패의 원인은 위 loop/zero sample의 source 분석이며, 관측 시간 자체를 무한 실행 증명으로 표현하지 않는다.
- 기존 finite-positive validation/phase scheduling 보완 범위에 증거를 합쳤으며 신규 티켓 수로 중복 집계하지 않는다. 정상 기본 설정이 hang한다는 증거는 아니다.
- Acceptance 보강: zero/negative/nonfinite dwell과 rate는 loop 진입 전에 명시적으로 거부하고, valid extremes에서도 phase clock이 진전하는지 검증한다. expected-invalid case가 CI를 무기한 점유하지 않도록 격리 timeout control을 둔다.
