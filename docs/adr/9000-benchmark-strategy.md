# 9000. 벤치마크 전략 — control-plane 성능공학 (SOTA)

- 상태: Proposed
- 날짜: 2026-06-04
- 결정자: Song Min
- 번호대: 9xxx = north-star / aspirational. 순차 결정이 아니라 "지향점"으로 둔다.
- 선행: [0001 — Feature-sliced 헥사고날 아키텍처](0001-hexagonal-feature-sliced-architecture.md)

## Context

`taskmesh`는 모든 task 앞단에 끼는 governed execution control-plane이다. 따라서
벤치마크의 목적은 "라이브러리가 빠른가"가 아니라 다음 세 가지를 *증명 가능한
수준*으로 특성화하는 것이다.

- **Q1. Governance tax** — task 하나당 거버넌스가 얹는 비용
- **Q2. Scalability** — 단일 `Mutex<State>`가 코어 증가에서 언제 무너지나
- **Q3. Behavioral fidelity** — fairness/retry-after/reduce가 *설정대로* 동작하나

control-plane을 순진한 closed-loop 루프 + 평균값으로 재면 coordinated omission,
큐잉 은닉, 머신 노이즈로 인해 숫자가 거짓이 된다. 우리는 production gate로 쓸
신뢰 가능한 숫자가 필요하다. 그래서 업계 SOTA 성능공학 관행을 채택한다.

이 레포의 구조적 사실들이 측정 설계를 직접 규정한다.

- 헥사고날 분리로 `taskmesh-engine`은 런타임 없이 단독 실행 → 거버넌스 비용을
  스케줄러 노이즈 0으로 격리 측정 가능.
- `admit` 성공 경로는 단일 전역 `Mutex<GovernedState>`를 **한 번** 잡고
  ([`governor.rs` `admit_inner`](../../crates/taskmesh-engine/src/engine/governor.rs)),
  `GovernedState::grant()`에서 `root_operation_id.to_string()`·`class`/`stage` clone·
  `permits` 삽입을 한다 → **전역 락 직렬화**와 **admit당 할당**이 핵심 변수.
  (drop→재락 패턴은 `abandon()`/timeout cold path에만 있고 admit hot path가 아니다.)
- `Clock` port가 이미 존재 → 시간 의존 로직을 결정화하고 syscall 노이즈를 제거 가능.
- AGENTS 원칙(deterministic, fail-closed)이 곧 행동 벤치의 합격 기준.

## Decision

벤치마크를 9개 기둥(pillar)으로 설계한다. 각 기둥은 "SOTA 기법 → 이 레포에서
무엇을 재나 → 도구"로 구체화한다.

### P1. Coordinated-omission-free 측정 모델

- **Open-loop load generation.** 클라이언트가 응답을 기다렸다가 다음 요청을 보내는
  closed-loop는 큐잉 지연을 은닉한다. *의도된 송신 시각(intended send time)* 을
  미리 정한 스케줄에 따라 요청을 주입하는 open-loop를 기본으로 한다.
- **Coordinated omission 보정.** 측정 지연이 도착 간격보다 길면 누락된 가상 요청을
  복원해야 한다. `hdrhistogram`의 `record_corrected(value, expected_interval)`로
  p50/p99/p999/p9999를 보정 기록한다. 평균은 보고하지 않는다.
- **Throughput–latency 곡선.** 단일 숫자가 아니라 offered load를 sweep하여
  p99 latency vs goodput 곡선(하키스틱)을 그리고 **knee(포화점)** 와 **SLO-bounded
  max sustainable throughput** 을 보고한다. 이것이 control-plane 특성화의 표준이다.

### P2. 2-트랙 지표 — 현실(wall-clock) + 결정성(instruction count)

| 트랙 | 도구 | 성격 | 용도 |
|---|---|---|---|
| wall-clock | `criterion` | 노이즈 있음, bootstrap CI | 실제 ns/op, 곡선 |
| instruction/cache count | `iai-callgrind` (Cachegrind/Callgrind) | **노이즈 0, 머신 독립** | CI 회귀 게이트 |
| allocation | `dhat` (또는 divan alloc) | 결정적 | alloc/op, bytes/op |

- wall-clock은 현실을 재지만 CI에서 흔들린다. **회귀 게이트는 명령어 카운트로** 잡아
  머신 독립·재현 가능하게 한다(예: `admit_success` instructions +2% 이상 → fail).
- `grant()`의 `to_string()`/clone/`BTreeMap` 삽입은 alloc/op로 결정적 추적.
  **게이트 기준은 baseline(현재 N allocs/op) 대비 회귀 0**이지 *0-alloc이 아니다*.
  현재 성공 경로는 `to_string()`+permit record 삽입으로 구조적으로 할당하므로, 0-alloc은
  저장 모델 재설계(root id interning, `Arc<str>`)를 요구하는 **별도 아키텍처 결정**으로
  분리한다 (→ 향후 ADR). 벤치 회귀와 아키텍처 변경을 혼동하지 않게 한다.

### P3. 확장성 모델링 — Universal Scalability Law

단일 `Mutex<State>`가 Q2의 본체다. thread sweep(1·2·4·8·16·코어수)에서 admit/release
throughput X(N)을 측정하고 **USL**을 피팅한다.

```
        N
X(N) = ─────────────────────────   (Gunther)
       1 + α(N−1) + βN(N−1)
```

- α(contention, 직렬화 비율) — 모든 governor 연산(admit/release/promote/snapshot)을
  직렬화하는 **단일 전역 `Mutex<GovernedState>`** + 락 보유 중 `grant()` 할당의 직접 비용.
  (admit이 락을 두 번 잡아서가 아님 — 초기 가설 정정, live code는 admit당 1락.)
- β(coherency, crosstalk) — 캐시라인 bouncing.
- 산출물: **N_max(처리량 정점)** 과 그 이후 코어를 더해도 *느려지는* 지점 예측.
  `usl` crate로 피팅. T03 이후 "락 1회"·sharded counter·lock-free 개선의 정량 근거가 됨.

### P4. 워크로드 현실성 — arrival process + popularity + trace replay

uniform 합성 부하는 거짓 안정성을 준다. 실제 retrieval/indexing 부하를 모사:

- **arrival**: Poisson(λ) 및 burst용 MMPP (`rand_distr::Exp`).
- **class popularity**: Zipfian (`rand_distr::Zipf`) — 소수 hot class가 현실.
- **trace replay**: 기록된 (도착시각, class, source, scope) 트레이스를 재생하는 모드.
  합성과 trace를 같은 harness가 처리.
- 모든 fixture는 `taskmesh-bench/src/workload.rs` 단일 출처. **T10의 12개 proof
  시나리오 테스트와 동일 fixture를 공유** → 벤치 = 실행 가능한 성능 acceptance.

### P5. Control-plane 전용 행동 지표 (Q3)

scheduler 성능공학의 표준 지표를 채택한다(처리량/지연만으로 불충분).

| 지표 | 정의 | 도구 |
|---|---|---|
| **Goodput** | admit→완료된 요청률 (단순 throughput과 구분) | harness |
| **Jain's fairness index** | 다중클래스 실현 share의 공정성 (0~1) | 산식 |
| **Reject ratio @ overload** | offered≫capacity에서 fail-closed 비율 | harness |
| **Queueing delay** | admit~dispatch 사이 대기 (P1 보정) | hdrhistogram |
| **Tail amplification** | p99/p50 비 — 과부하에서 꼬리 증폭 | hdrhistogram |
| **Work-conservation** | runnable 있는데 idle인 시간 (스케줄러 낭비) | harness |
| **Reduce determinism** | 입력 순서/seed를 흔들어도 byte-identical | `proptest` |

fairness는 ad-hoc 비교 대신 **Jain index**로, reduce 결정성은 다수 seed/순열에 대한
**property-based** 동치 검증으로 측정한다(결정성 위반 = 벤치 fail).

### P6. 동시성 정확성 병행 (perf와 co-locate)

빠른데 틀리면 무의미하다. 핫패스 perf 벤치 옆에 동시성 모델체킹을 둔다.

- **loom** — admit/release/seq의 atomic·락 인터리빙 **전수 탐색**(작은 상태).
- **shuttle**(AWS) — 더 큰 상태의 랜덤 인터리빙.
- 둘 다 `taskmesh-engine`의 `#[cfg(loom)]` 경로로 게이트.

### P7. 환경 위생 (측정 무결성)

- 스레드 **core pinning**(affinity), warmup 폐기, 머신 quiesce.
- **권위 있는 숫자는 전용 Linux 러너에서**: `isolcpus`/`cset shield`로 CPU 격리,
  `performance` governor + turbo off로 주파수 고정, SMT off, instruction-count 측정 시
  ASLR off.
- ⚠️ 개발 플랫폼이 darwin인데, macOS는 isolcpus·governor 제어가 사실상 불가하고
  turbo·QoS가 개입한다. **macOS는 directional dev-only**, 게이트 숫자는 Linux 러너에서만
  인정한다. macOS에선 `taskpolicy`/QoS 고정으로 변동을 줄이되 절대값은 신뢰하지 않는다.

### P8. 지속적 회귀 인프라

- `criterion --save-baseline`으로 baseline 저장, PR에서 **상대 회귀**만 게이트
  (절대값은 러너 의존). 명령어-카운트(P2)는 절대 게이트 가능.
- `github-action-benchmark` 또는 `bencher.dev`로 시계열 추적·알림.
- 회귀 감지 시 자동 **flamegraph**(`pprof-rs`/`cargo-flamegraph`) 첨부 + Linux `perf stat`
  하드웨어 카운터(instructions, cache-misses, branch-misses) 덤프.

### P9. 신뢰 가능한 baseline (비교군)

SOTA 벤치는 항상 credible baseline을 갖는다. governance의 *가치와 비용*을 동시에 보이려면:

- **raw tokio** (`spawn_blocking`/직접 await) — governance tax의 분모.
- **Semaphore-only governor** — 정원만 있는 순진한 거버너 대비 우리 거버넌스의 추가비용.
- **tower `ConcurrencyLimit`/`load-shed`** — 업계 표준 미들웨어 대비 포지셔닝.

## 산출물 구조

```
crates/
├── taskmesh-engine/benches/
│   ├── admit_release.rs        # criterion + iai-callgrind: 핫패스 ns/op + instr count
│   └── contention.rs           # thread sweep → USL 피팅 입력
├── taskmesh/benches/
│   └── governance_tax.rs       # e2e vs raw tokio / semaphore / tower (P9)
└── taskmesh-bench/             # publish=false, 무거운 dev-dep 격리
    ├── src/
    │   ├── workload.rs         # arrival(Poisson/Zipf)·trace replay·공유 fixture (P4)
    │   ├── loadgen.rs          # open-loop, coordinated-omission-free (P1)
    │   └── metrics.rs          # Jain index, goodput, tail amp, USL fit (P3/P5)
    └── benches/
        ├── retrieval_saturation.rs
        ├── multiclass_fairness.rs
        ├── overload_stability.rs
        └── composite_reduce.rs
```

## SLO 카탈로그 (초기 목표 — Linux 러너 기준, 측정 후 확정)

| 항목 | 목표 | 게이트 |
|---|---|---|
| `admit_success` instr/allocs | baseline 대비 회귀 0 (0-alloc 목표 아님) | iai 절대 게이트 |
| governance tax (e2e, no-op) | raw 대비 < X% | criterion 상대 |
| admit/release throughput | USL N_max ≥ 물리코어 | USL 피팅 |
| Jain fairness index (WFQ) | ≥ 0.95 | 행동 게이트 |
| reduce determinism | 100% (proptest) | 정확성 게이트 |
| reject latency @ overload | p99 < admit_success ×2 | hdrhistogram |
| queue bound @ overload | 무한 증가 0 (fail-closed) | 불변식 |

## 단계화 (현 skeleton 기준)

- **Phase 0 (지금)**: `admit_release` + `contention` (criterion + iai). admission 핫패스
  baseline·USL α 확보. 이후 T03~T06 회귀 게이트.
- **Phase 1 (T03~T06 병행)**: 슬라이스별 행동 벤치(fairness/memory/composite)를 proof
  fixture와 공유. loom/proptest 동반.
- **Phase 2 (T07~T09)**: e2e tax + rayon vs spawn_blocking, open-loop loadgen 전면 가동.
- **Phase 3 (T10)**: CI 회귀 인프라·시계열 대시보드·flamegraph-on-regress.

## 구현 현황 (2026-06-04)

`crates/taskmesh-bench`에 구현된 것과 아직 아닌 것을 명시한다(doc-impl 정합). 모든
벤치/예제/하니스는 컴파일·실행되며, 라이브러리 + 하니스 단위테스트와 e2e
`tests/hellgate.rs`가 green이다.

**구현됨**

- P1 — `LatencyRecorder`(hdrhistogram `record_correct`, 거부 샘플은 `dropped()`로 표면화,
  삼키지 않음), `loadgen::simulate`(open-loop 이산사건 시뮬레이터, 보존성 `is_conserved`).
- P3 — `metrics::fit_usl`(USL 최소제곱) + `is_in_domain`(유효역 판정) + `argmax_throughput`
  (경험적 peak). `examples/usl_probe`가 retrograde(역scaling)를 정직하게 보고.
  *측정 결과*: 단일 전역 Mutex는 N=1에서 peak, 이후 throughput 하락(α>1, contention-bound).
- P2 instruction-count — `benches/iai_governance.rs`(iai-callgrind, `iai` feature·Linux/
  valgrind). admit_release/reject/snapshot의 **결정적 명령어 카운트**. alloc track은
  `tools/bench-gate.sh`가 baseline(5 allocs/op) 회귀를 cross-platform으로 게이트.
- P4 — Poisson(`Exp`) + Zipf 도착, seed 고정. **trace record/replay** 구현
  (`workload::{trace_to_csv,trace_from_csv}`): 무손실 round-trip + replay가 시뮬레이션을
  byte-identical 재현. (MMPP burst는 여전히 미구현.)
- P6 — **loom 전수 인터리빙**(`taskmesh-engine/tests/loom_governance.rs`)으로 admit/release
  동시성 설계(Relaxed id-gen + single mutex) 안전성 검증 + **실제 Governor 스트레스**
  (`concurrency_stress.rs`: 8스레드 permit 유일성, max_inflight 캡 불변).
- P5 — goodput / Jain index / reject ratio / tail amplification. fairness는 *promotion
  순서의 윈도우*로 채점(최종 합계가 아니라) → WFQ 4:1을 [40,10]으로 검증. equal-weight
  Jain≥0.95를 벤치에서 게이트.
- P9 — `governance_tax`가 governed vs raw `spawn_blocking` vs semaphore-only 대조.
- 결정적 측정 규약: counter clock(`ManualClock`), no-op body, alloc/op 프로브
  (`examples/alloc_probe` → **admit+release = 5 allocs/op**, baseline 게이트용).

*측정 스냅샷*(dev darwin, 비권위): admit+release ≈ 320ns, unknown-class reject ≈ 48ns,
overload 4×에서 큐가 depth에 고정되고 reject가 74%를 흡수(fail-closed).

- P8 — `.github/workflows/bench.yml`: alloc-gate(blocking) + instruction-count(iai,
  baseline 캐시 비교) + loom 잡. Justfile `bench`/`bench-gate`/`bench-iai`/`loom` 타겟,
  `gate`에 alloc 게이트 편입.

**미구현 / deferred** (이 ADR이 기술하나 아직 코드에 없음 — 혼동 방지)

- P4 MMPP/burst 도착, P5 work-conservation 지표 — **Phase 2**.
- P7 환경 위생(core pinning/isolcpus) + 전용 Linux 러너 — 러너 설정. **Phase 3**.
- P8 시계열 대시보드(github-action-benchmark/bencher.dev), flamegraph-on-regress — **Phase 3**.
- reduce 결정성 proptest — 진행 중(엔진 crate, proptest dep 추가됨).

SLO 카탈로그 중 현재 *게이트로 강제*되는 것: 큐 bound(fail-closed), 보존성, latency
드롭 0, equal-weight Jain≥0.95, USL 복원, alloc/op ≤ baseline, 동시성 안전성(loom +
실코드 스트레스), instruction-count 회귀(CI). 나머지(tax %, N_max≥cores)는 측정되나
절대 게이트는 Phase 3 인프라 대기.

## Consequences

### 긍정
- 헥사고날 분리 덕에 거버넌스 비용을 런타임 노이즈 0으로 격리 측정 → tax 숫자가 정직.
- 명령어-카운트 게이트로 CI가 머신 독립·재현 가능. 평균/closed-loop의 거짓 안정성 제거.
- USL 피팅이 단일 Mutex의 한계를 *예측*으로 만들어 락 개선의 ROI를 정량화.
- 벤치 fixture = proof 시나리오 fixture → 성능과 정확성이 같은 입력에서 검증.

### 비용 / 트레이드오프
- 도구 스택이 크다(criterion·iai-callgrind·hdrhistogram·dhat·loom·shuttle·proptest·usl).
  `taskmesh-bench`에 격리해 라이브러리 crate 의존은 최소 유지.
- 권위 있는 숫자에 전용 Linux 러너가 필요. macOS dev 숫자는 directional only.
- iai-callgrind는 valgrind 필요(Linux/CI). 로컬 macOS에선 wall-clock·alloc 트랙만.

### Non-goals
- 마이크로 최적화 자체가 목적이 아니다. 게이트는 *회귀 방지*와 *특성화*용이다.
- 분산/멀티노드 부하는 범위 밖(이 control-plane은 in-process).

## Alternatives

- **criterion 단독** — 간단하나 coordinated omission·머신 노이즈·확장성 모델 부재로
  control-plane 게이트엔 불충분. 기각(P1/P2/P3 미달).
- **divan 단독** — 가볍고 alloc 카운트 내장이나 회귀 인프라/보정 측정 약함. alloc 트랙
  보조로만 채택 가능.
- **closed-loop wrk/벤치류** — 큐잉 은닉. control-plane엔 부적합.

## References

- Gil Tene, *How NOT to Measure Latency* — coordinated omission.
- Neil Gunther, *Guerrilla Capacity Planning* — Universal Scalability Law.
- Jain, Chiu, Hawe (1984) — fairness index.
- `iai-callgrind`, `hdrhistogram`, `loom`, `shuttle`, `dhat`, `usl` (crates).
- [ADR 0001 — 아키텍처](0001-hexagonal-feature-sliced-architecture.md), [RFC 0001](../rfcs/0001-governed-runtime.md)
