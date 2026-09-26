# 9000. Taskmesh 벤치마크 측정 전략

- 상태: Proposed
- 최초 날짜: 2026-06-04
- 재정리: 2026-09-26
- 결정자: Song Min
- 상세 실행 계획: [Sep-25 벤치 티켓](../plans/sep-25-taskmesh-benchmark/tickets/README.md)
- 주장 사전등록: [claim-contract.json](../../tools/bench/scenarios/claim-contract.json)

이 ADR은 측정 방법의 경계를 정한다. 현재 소스에는 업계 SOTA 성능 결과나 자격화된
host p99/goodput이 없다. 6월 초안의 `5 alloc/op`, 자동 대시보드·flamegraph,
`< X%` governance tax, 절대 Linux 전용 판정, host 지연에 대한 일괄 histogram
보정은 현재 구현 또는 승인된 성능 문턱으로 취급하지 않는다. 이전 초안은 Git 이력에
남아 있다.

## 대상

Taskmesh는 단일 프로세스의 governed execution library다. 측정 대상은
`Governor`의 admission/claim/release/snapshot과 public `TokioRuntime`의
`run_io`, `run_blocking`, `run_cpu`, `run_local`, requested-stack 경로다. HTTP,
데이터베이스, 분산 큐, 검색 품질은 Taskmesh 자체 성능 분모가 아니다. work body의
시간과 거버넌스·호스트 비용을 분리한다.

## 결정

| 레일 | 측정값과 용도 | 현재 권한 |
|---|---|---|
| Governor micro | 명명된 상태 전이의 ns/op, alloc/op, Linux IAI instruction count. setup과 완료 연산 분모를 밝힌다. | 해당 연산·fixture의 비용만 설명한다. host 응답 지연은 설명하지 않는다. |
| 결정적 정책 모델 | seeded arrival, admission/queue/promotion/terminal 순서, 가상 admission wait와 보존식. | 정책 oracle이다. 시뮬레이터 실행 시간과 가상 wait를 실제 host p99로 부르지 않는다. |
| Public host | 미리 검증한 유한한 intended-send 스케줄을 실제 public `run_*` 호출에 주입한다. class/path별 응답, worker finish, custody, SLO-goodput을 원시 행에서 복원한다. | 이 레일만 host latency/goodput을 낼 수 있다. 현재 구현은 진단용이며 성능 자격은 없다. |
| 성능 자격 | 동일 머신의 반복 측정, generator/observer 검증, 소스·바이너리·fixture 식별자, 실패 실행 포함 원시 영수증, 실행 단위 불확실성. | 별도 고정 호스트 수동 또는 예약 레일. 일반 PR CI의 단일 wall-clock 수치는 판정자가 아니다. |

### 도착과 모집단

- Open-loop overload에서는 각 *의도된* 요청에 raw 행을 하나 둔다. 생산자 cap 또는
  지연으로 제출하지 못한 요청은 `not_submitted`로 남겨 generator-limited rate
  point를 거절한다. 별도 closed-loop fixed-concurrency 결과와 p99 모집단을 합치지
  않는다.
- 기본 지표는 선언된 class/path SLO 안에 성공한 injection cohort 요청 수를
  injection-window 초로 나눈 **SLO-goodput**이다. 같은 분자를 전체 intended
  arrivals로 나눈 비율, completion fraction, typed reject/cancel/deadline/drop/unanswered를
  함께 보고한다. 성공 응답에만 조건부 p50/p95/p99를 붙이고 표본 수와 정밀도 부족을
  명시한다. 응답이 없는 요청에 0 지연을 넣지 않는다.
- caller 응답·drop과 실제 body finish/lease 해제는 별도 원장이다. drain과 최종
  Snapshot 보존식이 성립해야 한다. Snapshot의 주기 표본 최대치는 정확한 high-water가
  아니라 하한이다.
- 전체 intended 스케줄과 누락 행이 raw로 보존되므로 그 응답 표본에 임의의
  `record_corrected` 값을 더하지 않는다. histogram 보정은 실제로 샘플링이 생략된
  별도 실험의 추정법이며, 미제출·무응답을 관측된 완료로 바꾸지 못한다.

### 실행 유효성

- 사전등록 파일에서 주장, SLO, completion floor, 절대 rate grid, 비교군,
  minimum detectable effect, 제외 규칙을 측정 전에 고정한다. 아직 비어 있는 값은
  `blocked` 상태로 남긴다. 합성 부하를 대표 소비자 부하로 이름 붙이지 않는다.
- null-work generator headroom, 동일 바이너리 A/A, recorder/Snapshot on-off,
  외부 저주기 CPU/RSS/thread 샘플러를 측정해 producer 또는 observer 왜곡을
  구분한다. 측정 길이와 반복 횟수는 pilot 분산과 목표 효과에서 정한다.
- generator headroom의 구조 검사에서는 동일 finite 스케줄을 같은 injection
  window 안에서 정확히 두 번 재생한 별도 scenario/raw를 요구한다. target과 같은
  rate의 generator 실행은 headroom 증거가 아니다. 이 2배 replay도 허용 lag와
  미제출 비율의 사전등록·실측 판정 없이는 자격화되지 않는다.
- baseline과 candidate의 source revision만 선언된 비교 변수로 둘 수 있다.
  body, SLO, class/feature/topology, host, rate grid, raw 모집단, 제외 규칙이
  다르면 비교를 거절한다. 요청별 상관 샘플을 독립 반복으로 세지 않는다.
- raw Tokio, Semaphore, Tower는 더 좁은 **mechanism control**이다. 동등한
  admission·bounded queue·deadline/cancel·worker custody·drain 계약이 입증된
  named peer만 업계 비교군에 넣거나 주장을 공통 교집합으로 좁힌다. 업계 주장은
  독립 재실행이 필요하다.

## 현재 구현과 남은 경계

- 기존 Criterion/IAI와 allocation/smoke gate는 named micro/컴파일 범위다.
  현재 allocation 설정은 `tools/bench/perf-gate.json`의 `8 alloc/op`다. IAI 최초
  baseline 생성은 회귀 PASS가 아니다.
- B01 Criterion 사례는 `governance_tax`의 full-facade(호출 중 spec 구성)와
  prebuilt-spec(측정 밖 spec/runtime clone)을 구분한다. Tokio/Semaphore/Tower는
  좁은 mechanism control이다. `queue_scaling`은 class 수 1/8/32와 class당
  queue 깊이 1/16에서 fixture 구성·queue 적재를 제외하고 holder release 한 번과
  promotion 한 건을 잰다. `cold_host_lifecycle`은 구성, 첫 IO 호출, warm IO 호출,
  빈 host drain, IO와 blocking 각각 동일 경로·body의 완료된 1/10건 batch를 별도 측정한다.
  Criterion batch 지연의 분모는 batch 하나이며, 건당 값은 그 지연을 1 또는 10으로
  나누어 계산한다. 이 사례의 smoke는 상태·실행 가능성 증거이지 고정 host 성능
  영수증이 아니다. `*_sim`의 Criterion 시간은 simulator 계산 시간이다.
- `taskmesh-bench`의 finite host runner는 Send IO/blocking/default CPU 및
  requested-stack blocking/async를 실행하고
  raw v2에 intended, submitted, caller terminal/drop, body start/finish, cut/settlement,
  sampled Snapshot과 실패 시 `invalid` 원시 행을 기록한다. Rust typed
  scenario/raw/Builder 검증과 Python 구조 검증이 연결돼 있다.
  Bench-owned class 선언은 FIFO 또는 양의 WFQ weight를 명시할 수 있다.
  H3의 4:1 fixture 실행은 정책 전달·raw 정산 smoke이며 host 공정성 판정은 아니다.
- `just bench-host`는 진단 raw/summary, 보관 실행 파일, 같은 실행의 resolved
  topology와 실행 출처 sidecar를 만든다. 실행 전후 소스·호스트·전원 상태,
  빌드 명령/환경/feature, 보관 파일과 fixture/raw/topology digest가 기록된다.
  검증기는 실행 파일 bytes를 다시 해시하고 같은 시나리오로 만든 런타임의
  topology와 대조한다. 이는 동일 소스의 독립 재빌드나 성능 자격을 뜻하지 않는다.
  H4 requested-stack blocking/async의 Python raw 분석 경로를 Rust scenario와
  일치시켰으며, 두 경로를 포함한 실제 CLI raw→summary→검증 왕복은 진단 범위에서
  통과했다.
  `closed_loop.rs`는 별도의 고정 caller 동시성 완료량 경로다. 각 slot은 직전
  응답 뒤에만 다음 호출을 제출하고 요청 수·최대 실행 시간을 제한한다. 독립 raw
  schema에는 외생 intended-arrival 시각이 없으므로 open-loop의 overload tail이나
  intended-arrival SLO-goodput과 합치지 않는다.
  `local_host.rs`는 current-thread LocalSet에서 `!Send` payload를 실행하고,
  같은 caller thread의 finite pacer 지연/미제출을 별도 raw에 남긴다. Snapshot,
  deadline, cancel/drop은 이 진단 schema에서 지원하지 않으며 IO 경로로 대체하지
  않는다. local schema v2는 미제출 offer도 pacer 관측 시각을 보관하고 lag와
  재계산해 대조한다. v1 raw는 이 검사를 제공하지 않으므로 v2 증거로 승격할 수 없다.
  비교 측정은 없다.
  `composite_host.rs`는 H6에서 부모의 reduce 선언과 별도 공개 호출로 제출한
  IO/blocking/CPU 자식 세 건을 진단한다. 한 자식의 작업 오류에도 caller가
  성공 키를 정렬해 합치며, 자식별 시각·결과, class 정산, root attribution 소멸을
  독립 raw schema로 검사한다. reduce 실행 주체는 caller다. 이 단일 smoke는
  fan-out 성능이나 내부 reducer 구현을 입증하지 않는다. H6 반복 고정 host 측정은 없다.
  이 변경은 부모 timeout/실패 시 자식별 부분 관측과 drain 이후 상태를
  별도 `invalid` raw에 보존하고 typed validator로 구조를 확인한다. 실패 실행은
  성공 영수증으로 승격하지 않는다.
  `just bench-host-special`은 이 세 개의 독립 진단 schema를 소스 내용,
  빌드 feature, 보관된 실행 파일과 별도 typed validator, raw, topology, process
  resource digest에 묶는다. 빌드/실행 중 소스가 바뀌면 거부하고 실패 이유를 남긴다.
  `just bench-host-special-verify`는 보관 파일로 재검증한다. dirty-source 구조
  영수증도 `UNQUALIFIED`이며 open-loop 성능 영수증으로 입력할 수 없다.
  현재 calibration 파일의 boolean은 실측 증거가 아니므로
  `host_perf.py --require-performance`는 fail closed다.
  동일 window의 2배 generator replay와 H0–H8 지원 현황 인덱스는 진단 범위로
  구현됐다. acquisition wrapper는 feature 선택을 모든 control 실행에 전달하며
  default/Rayon 영수증의 build feature 신원을 분리한다. 실측 calibration,
  external resource sampler의 왜곡 검증,
  `host_compare.py`는 동일 absolute rate에서 baseline/candidate 영수증을
  재검증하고, 독립 실행쌍의 SLO-goodput 차이와 설명용 bootstrap 범위를 출력한다.
  불완전 실행, 바뀐 소스/host/feature/topology/workload, 재사용·중첩된 실행과
  불균형 순서를 거부하며 실패 보고서를 보존한다. 이 도구는 항상
  `UNQUALIFIED`를 출력한다. 실측 보정 및 고정 host 반복 실행,
  closed-loop/local/H6의 반복 비교 측정, Rayon 비교 측정과
  대표 H7은 남아 있다.
  비교 보고서는 실행별 외부 CPU 표본 delta와 RSS/thread 표본 최대치,
  표본 구간을 하한 관측치로 함께 보존한다. 표본이 한 개뿐이면 CPU delta는
  `null`이며 자원 효율 우위를 주장하지 않는다.
  별도 sampler on/off 균형쌍은 동일 시나리오·소스·바이너리에서 외부 자원
  관측의 영향을 진단한다. control-budget evaluator는 시나리오·clean HEAD에
  묶인 예산으로 generator lag/미제출, A/A, Snapshot, recorder 응답수 및
  sampler on/off SLO-goodput 및 성공 응답 p99 변화를 계산한다. A/A와 Snapshot도
  동일하게 p99 변화와 각 class/path의 최소 성공 표본 수를 검사한다. 예산 통과도 `UNQUALIFIED`이며
  evaluator는 검증한 control bundle의 scenario·bundle·resource digest를 다시
  대조해 재열람 사이의 artifact 교체를 거부한다. B00 계약과 반복 고정 host 결과를
  대체하지 않는다.
  target 및 모든 control arm의 미제출·settlement 미응답·producer lag·늦은
  Snapshot을 검사하고 문제 arm을 별도로 보고한다. Minimal recorder는 typed raw가
  Snapshot을 금지하며 latency를 만들지 않는다. Snapshot·recorder·sampler의
  비교 예산에는 양방향 실행 순서가 같은 수의 쌍만 입력할 수 있다.
  control bundle은 최종 검증 후 게시한다. 최종 검증 실패도 raw를 유지한
  `incomplete` bundle로 남긴다. Python acquisition 산출물과 보관 실행파일은
  같은 디렉터리의 private 임시 파일을 hard link로 게시해 기존 파일을 원자적으로
  덮어쓰지 않는다. 이 계약은 crash durability 영수증을 뜻하지 않는다.
  구조 bundle v3는 target Snapshot-off 또는 control과 같은 cadence의 Snapshot-on을
  허용한다. Snapshot-on target을 쓸 때 A/A는 on workload, recorder full/minimal은
  같은 workload의 off 파생 시나리오를 사용하고 그 digest를 묶는다. On target의
  여덟 프로세스 진단 smoke는 통과했지만 관측 비용 예산을 입증하지 않는다.
  상세 목적·파일·DoD는 위 티켓이 소유한다.
- `just bench-gate`는 allocation, `just bench-smoke`는 bench 실행 smoke,
  `just bench-iai`는 Linux의 지정된 instruction 사례다. 어느 것도 host 성능
  PASS나 업계 SOTA 자격을 대신하지 않는다.

## 기존 P1–P9 참조

기존 코드 주석의 번호는 P1=도착·기록, P2=micro/할당·명령어, P3=확장성 모델,
P4=워크로드, P5=행동 지표, P6=동시성 oracle, P7=측정 환경,
P8=회귀 인프라, P9=mechanism control을 뜻한다. 번호는 출처 추적용이며
각 기둥의 구현 완료나 성능 자격을 표시하지 않는다.

## Consequences

측정 비용은 늘지만 각 숫자의 분모와 계약이 검토 가능하다. 좁은 CI oracle과
고정 호스트 성능 자격은 별도 결과로 보관한다. 원시 행·fixture·실행 출처가
누락되거나 correctness oracle이 실패한 rate point는 결과에서 조용히 제거하지
않고 실패 사유와 함께 보존한다.
