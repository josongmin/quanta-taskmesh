# 실행 순서·병렬화·파일 소유권

## 역할과 권한

- I: 통합/계약 owner
- E: Engine owner
- R: Runtime/adapter owner
- F: Fairness owner
- V: 독립 검증 owner
- B: Benchmark owner
- C: CI/gate owner
- D: Dependency/toolchain owner
- Q: PM/문서 owner

역할은 계획상 책임이다. 실제 assignee는 실행 시작 시 지정한다. `depends_on`은 구현 통합/완료 선행 관계이며 read-only 조사·reference/fixture 준비를 막지 않는다. 티켓의 `locks`는 변경 가능한 범위의 예약 요구다. 같은 lease는 어떤 역할/티켓이든 동시에 1개만 보유한다. 의존성이 없다는 이유만으로 공용 파일을 동시에 쓰지 않는다.

## 배타적 적용 권위

| Lease / 범위 | 적용 권위 | 인계 규칙 |
|---|---|---|
| contract — crates/taskmesh-contract | I | E/R/F는 변경 spec과 compatibility tests 제공; I가 DTO/public API 적용 |
| engine — shared state/scheduler/accounting | E | F는 H16-007 동안 명시적 lease를 받아 적용; I의 H16-010 통합도 같은 lease 양도 |
| host — crates/taskmesh runtime/worker | R | H16-008/009/011/012 직렬; H16-010은 R→I→R 인계 |
| rayon — crates/taskmesh-rayon | R | H16-002 interface 변경도 I/E와 합의한 R 적용 |
| manifests — Cargo.toml/Cargo.lock/toolchain | D | 다른 lane의 module/feature/dependency 요청은 D가 직렬 반영 |
| bench — crates/taskmesh-bench | B | H16-015→016→017; 같은 bench 파일 병렬 수정 금지 |
| ci/gate-tools — Justfile, .github, tools/arch, tools/semgrep, bench gate helpers | C | B가 H16-017 metric/parser spec·tests를 준비하고 C가 gate helper/workflow 적용. H16-018과 동시 적용 금지 |
| pm — tools/pm | Q | 실제 생성 AGENTS 등 target은 별도 승인 전 write set에서 제외 |
| engine-tests/host-tests | 해당 production lease owner + V | model seam은 E/R; V는 독립 oracle/fixture. 동일 test module 충돌 시 단일 적용자 |
| docs | Q, 계획/qualification은 I | source 계약 확정 후 H16-021 docs migration; shared docs/release-checklist는 Q→I 인계 |
| qualification | I | immutable snapshot과 receipt 저장. CI 수정 필요 시 C에게 반환하고 새 SHA로 재검증 |

모든 production 파일의 broad lane 권위가 우선한다. 티켓의 기존 파일 목록은 review 범위이며 그 파일을 전부 독자 수정할 권한이 아니다. 신규 모듈 export도 소유자에게 인계한다.

## 작업 흐름

| 단계 | 실행 | 병렬 가능 / 종료 조건 |
|---|---|---|
| 0 | H16-001 / H16-015 / H16-019 | 계약/inventory, benchmark measurement, dependency graph 독립. shared Cargo 변경은 D만 |
| 1 | E: H16-002→003→004→005→006 | engine 단일 작성자. Q: H16-020, B: H16-015→016, D:019 병렬 |
| 2 | R: H16-008; 이후 H16-009→011→012 | 008은002 이후이나 engine 단계와 contract/host lease 충돌 시 대기. 009는006·008 완료 필요 |
| 3 | F: H16-007 / R: H16-011→012 | 007은006 이후. host 작업이 engine 파일도 건드리면 동시에 적용하지 않고 E 승인 순서로 직렬화 |
| 4 | I: H16-010→E:013→V:014 | 010은007·009·012 모두 완료 후. physical dispatch+fairness 통합과 실제 regression |
| 5 | B/C: H16-017→C:018 | 017은016·019 이후. 018은014·017·019·020 완료 후 전체 gate parity proof |
| 6 | Q: H16-021→I:022 | 품질 disposition·문서 migration 뒤 immutable source qualification |

이 표는 wall-clock 일정 추정이 아니다. task dependency DAG는 [plan.json](plan.json)이 기준이고 overlapping write lease가 추가 직렬 조건이다. 초기 P1 장애 containment는 담당 티켓의 최소 substep으로 별도 review/receipt를 낼 수 있지만 관련 구조·regression 완료 없이 원본 finding을 닫지 않는다.

## 티켓 시작/인계 체크리스트

1. HEAD/dirty fingerprint/source file digest, active owner, lease를 기록한다. 다른 사람의 untracked audit/doc 파일을 소유했다고 간주하지 않는다.
2. blocking decision과 consumer compatibility를 확인하고 acceptance별 failing regression/독립 oracle을 준비한다.
3. 범위 내 구현 후 normal/negative tests를 실행한다. command exit만 아니라 expected assertion·test count·artifact identity를 확인한다.
4. shared file patch는 적용 owner에게 넘긴다. 최종 적용 source에서 재실행한다; lane 단독 green은 통합 green이 아니다.
5. source가 변하거나 manifest/feature가 달라지면 영향을 받는 receipt를 무효화하고 재검증한다.
6. heavy same-host build/benchmark는 qualification owner가 직렬 예약한다. CPU contention 결과를 성능 개선/회귀 증거로 사용하지 않는다.

## 변경·승인 중단 조건

public API breaking, PM 실제 target overwrite, 외부 consumer 수정, branch protection 변경, deploy/activation은 원래 문서 작성 요청의 범위가 아니다. 실행 시 필요한 권한을 별도로 확보한다. arbitrary external waitgraph detection, forced thread kill, 새 DAG scheduler 요구가 나오면 별도 RFC로 분리한다.

