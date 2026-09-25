# S25-008 — 복합 admission·capacity 경계

- 상태: PLANNED; 우선: P1; 선행: [S25-001](S25-001-contract-boundaries.md), [S25-005](S25-005-executor-authority.md); 소유: engine/admission owner, host fixture는 host owner.
- 주 담당 시나리오: A05, B05, B20, H01, H02, H07, H32, D01.
- 근거: `crates/taskmesh-engine/src/features/admission/{mod,pending}.rs`의 primary blocker 순서, `Governor`의 단일 admission ledger, `taskmesh`의 resolved dispatch role/domain.

## 목적

class semantic quota와 worker/pool topology를 **동일 admission transaction**의 서로 다른 제약으로 판정한다. class/role/physical/CPU/memory의 독립 fixture를 조합해 blocker 우선순위와 정책 선택을 검증한다. queued와 accepted closure 모두 유한한 capacity를 점유하게 하고 executor 뒤 숨은 무한 대기를 만들지 않는다. class 정책은 product-neutral로 유지한다.

## 변경 파일

| 구분 | 파일 | 조치 |
|---|---|---|
| 신규 fixture | crates/taskmesh-engine/tests/hardening_admission_matrix.rs | class·capability·CPU·memory의 단독/복합 blocker, 0/1/limit 경계, fallback·scavenger 결정을 독립 event ledger로 판정. |
| 신규 host fixture | crates/taskmesh/tests/hardening_admission_host_capacity.rs | local/blocking/CPU role 및 shared physical domain의 실제 worker 점유를 barrier로 관측. S25-005 executor authority 결과를 소비. |
| 기존 회귀 | crates/taskmesh-engine/tests/hardening_exact_accounting.rs, pending_resolver.rs, hardening_fairness_reference.rs; crates/taskmesh/tests/substrate_enforcement.rs | 단독 제약 oracle과 비교하고 중복 fixture를 만들지 않음. |
| 조건부 source | crates/taskmesh-engine/src/features/admission/mod.rs, admission/pending.rs, engine/governor.rs, engine/state.rs, features/fairness/scheduler.rs | 재현된 잘못된 blocker·원장·promotion만 수정. |
| 조건부 host/docs | crates/taskmesh/src/runtime.rs, execution_plan.rs, builder.rs; docs/taskmesh-library-spec.md, taskmesh-external-interface.md | resolved role/domain 또는 공개 계약이 실제 동작과 다를 때 해당 owner가 수정. |

## 구현 순서

1. S25-001의 필드별 0 의미, primary blocker 우선순위, memory Queue 예외를 입력 표로 고정한다. 각 offer의 operation/root ID를 고유하게 둬 recursion 거절과 capacity 결과가 섞이지 않게 한다.
2. 같은 Governor에서 제약 하나씩 포화→두 제약 동시 포화→선행 blocker 반환→후행 blocker 노출을 수행한다. expected-held는 입력 permit별 비용을 별도 합산하고 snapshot의 누적값을 기대값으로 재사용하지 않는다.
3. admission/queue/claim/release/degrade마다 offered = terminal + live admitted + live queued가 유지되는 event ledger를 둔다. terminal은 reject, abandon, completion 중 실제 한 사건으로만 기록한다.
4. host에서는 executor closure를 barrier로 묶어 실제 시작/종료와 engine의 role·physical 점유를 비교한다. 두 번째 미계상 대기열이 드러나면 executor owner에게 반례를 넘긴다.
5. fixture가 실패하는 최소 전이에만 source를 수정한다. 새 engine pool, 제품별 class 의미, admission 앞 별도 queue는 추가하지 않는다.

## DoD

- [ ] `S25-008-A05` 서로 다른 class의 CPU/memory 합산을 permit별 독립 ledger와 대조하고 한 class 해제 후 다른 class의 진전을 확인한다.
- [ ] `S25-008-B05` class, role pool, CPU, memory 각각 단독 포화에서 정확한 blocker/verdict를 확인한다. Memory가 primary인 경우에만 memory Queue가 일반 Reject를 덮는다.
- [ ] `S25-008-B20` local role만 포화·다른 physical domain free 상태에서 `local_runtime` blocker와 무관 domain 무변화를 검사한다.
- [ ] `S25-008-H01` 여러 class×지원 substrate의 bounded burst/open-loop에서 offered·terminal·leftover를 독립 세고 class queue/pool/domain 상한을 보존한다.
- [ ] `S25-008-H02` class 여유와 shared physical domain 포화를 분리하고 두 class/role의 합산 charge와 정확한 domain blocker를 확인한다.
- [ ] `S25-008-H07` primary와 best-effort scavenger, fallback 경쟁에서 runnable primary 우선·fallback 재계상·drop의 명시적 terminal을 확인한다.
- [ ] `S25-008-H32` inflight+measured memory 동시 포화 → inflight 우선 Reject → holder 해제 후 memory-primary Queue를 같은 fixture에서 판정한다.
- [ ] `S25-008-D01` 0/1/한계/한계+1의 각 필드 의미를 표로 고정하고 invalid/unbounded/비queue를 혼동하지 않는다.

각 항목의 추가 판정:

- A05: 두 class의 CPU·memory 합이 budget 경계/1초과를 오갈 때 snapshot과 permit별 독립 합산이 일치하고 release 후 다른 class ticket이 정확히 한 번 진전한다.
- B05: 단독 blocker와 인접 쌍의 동시 blocker에서 전체 BlockerSet과 primary verdict를 따로 비교한다. inflight+memory 동시 포화에서는 inflight가 우선, 이를 풀면 memory-primary Queue가 된다.
- B20: local_runtime만 포화한 동안 무관한 physical domain의 occupancy가 변하지 않으며 local closure 시작 수가 slot을 초과하지 않는다.
- H01/H02: class별 offered/terminal/live ledger, queue 상한, 두 class가 공유하는 physical domain의 실제 동시 점유를 각각 관측한다. class 여유와 domain 포화를 혼동하지 않는다.
- H07: primary runnable 우선, fallback 비용의 정확한 재계상 1회, capability 유지, drop/reject의 단일 terminal을 확인한다.
- H32: inflight+measured memory 동시 포화→inflight 해제→memory-primary Queue→memory 반환 후 claim을 한 fixture에서 판정한다.
- D01: 0/1/limit/limit+1은 각 필드 타입의 계약에 따라 분리한다. invalid policy build reject와 valid nonqueue admission reject를 구별한다.

## 계획된 검증

- Owner-local: cargo test --locked -p taskmesh-engine --test hardening_admission_matrix; cargo test --locked -p taskmesh --test hardening_admission_host_capacity. 기존 exact accounting·resolver fixture도 영향 범위에 따라 실행한다.
- 통합: S25-012가 같은 clean HEAD에서 just test-core, just test, CI profile을 적용한다. 성능 수치와 host latency는 S25-011 소관이다. 이 계획 작성 중 테스트는 실행하지 않는다.

## 인계·중단 조건

- 인계: 정책/용량/예상 blocker 표, 입력 event ledger, host barrier trace, 실제 변경 파일, 공개 계약 영향.
- S25-005의 선언 authority가 정해지지 않았거나 hidden queue가 발견되면 상한 assertion을 완화하지 말고 executor owner에게 넘긴다. blocker 우선순위가 문서와 다르면 S25-001 계약을 먼저 재확인한다.
