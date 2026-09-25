# BG25-008 — compound admission과 capacity authority

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P1
- 선행: BG25-001, BG25-005
- 소유: engine/admission owner; host capacity fixture는 host owner

## 2026-09-25 재감사 잔여

- **저장소 코드:** A05/B05/B20/H01/H02/H07/H32/D01의 bounded capacity·policy fixture가 직전 clean HEAD `da5356b`의 `test` 게이트에서 PASS였다. 새 확정 결함 없음.
- **증거 한계:** 유한한 대표 조합과 독립 원장의 판정이며 모든 class×substrate 상태공간의 소진 증거는 아니다. 새 counterexample이 없으면 pool topology를 늘리거나 scheduler를 수정하지 않는다.
- **남은 증거:** 문서 변경 후 clean exact-source CI 재검증은 BG25-012가 소유한다.

## 목적

class, capability, CPU, measured memory, tier/fallback이 동시에 충돌할 때 원자성, blocker priority, boundedness를 independent event ledger로 증명한다.

## 근거

- 현재 intake는 모든 blocker를 계산하고 primary ordering을 적용한다.
- 기존 exact accounting과 single-dimension fixture에 `hardening_admission_matrix.rs`, 독립 input ledger, real-host `host_open_loop.rs`가 추가됐다.
- 이 oracle에서 추가 production mismatch는 관측되지 않았다. 성능 상한이나 배포 host 채택까지 증명하는 결과는 아니다.

## 변경 파일

- blocker/policy boundary fixture: `crates/taskmesh-engine/tests/hardening_admission_matrix.rs`
- independent input-ledger fixture: `crates/taskmesh-engine/tests/hardening_admission_ledger.rs`
- host overload fixture `crates/taskmesh/tests/host_open_loop.rs`
- 재현 시에만 `features/admission/{mod,pending}.rs`, `engine/{governor,state}.rs`
- policy/spec docs if precedence changes

## 작업 계획

1. 입력 event로 expected per-class/global/pool ledger를 별도 계산한다.
2. primary blocker 순서를 class→capability→CPU→memory→queued-behind 축에서 교차한다.
3. measured overcommit과 fallback을 포함하고 release/reconcile 후 진전을 검사한다.
4. 실제 실패한 transition만 최소 구조 수정한다.

## DoD

- `BG25-008-A05`: cross-class global CPU/memory totals and release progress match the independent ledger.
- `BG25-008-B05`: each single and selected compound blocker produces the accepted primary verdict and queue policy.
- `BG25-008-B20`: local-only pool saturation changes no unrelated physical occupancy.
- `BG25-008-H01`: bounded mixed-substrate overload has no hidden executor queue before permit.
- `BG25-008-H02`: free class quota plus saturated shared domain reports/queues on that domain exactly once.
- `BG25-008-H07`: primary, scavenger, and memory fallback preserve tier priority and re-account all resources.
- `BG25-008-H32`: class-full beats memory; after class release, a new memory-primary request follows memory queue policy.
- `BG25-008-D01`: 0/1/exact/+1 meanings for every capacity field are table-driven and unambiguous.

## 검증

- Engine matrix + focused host capacity fixture.
- Assertions include closure count, queue depth, per-class/global held, and every capability occupancy.

## 구현 증거 (2026-09-25)

- `hardening_admission_matrix`는 compound blocker 전체 집합, primary precedence, queue-policy 전환, rejected side-effect 0, 0/1/exact/+1 경계를 검증한다.
- `hardening_admission_ledger`는 입력 이벤트에서 독립 계산한 class CPU/memory, inflight, blocking-pool occupancy를 cross-class admit/release마다 대조한다.
- `host_open_loop`는 실제 host 응답과 worker custody ledger를 분리해 closure, queue, execution accounting을 검증한다.
- 이 oracle들에서 추가 production mismatch는 관측되지 않았다. 실행 결과는 최종 committed HEAD의 BG25-012 receipt에서 판정한다.

## 최종 의미 감사

- H01은 `1b1d8f6`에서 `hardening_mixed_overload::mixed_substrate_open_loop_burst_never_reaches_workers_before_permit`로 재매핑됐다. 이 case는 네 class/dispatch, capability·physical-domain 점유, 16건 동시 burst의 typed reject, worker side-effect 0, 반환 후 재사용을 assertion으로 확인한다. 대응 fixture와 assertion 후보가 추가됐으며 실행 결과와 전체 oracle 충족은 최종 HEAD 감사/receipt에서 판정한다.
- A05는 global CPU host contention과 cross-class memory reconcile/promotion의 input-derived ledger case를 함께 연결한다.
- B05는 class quota, capability pool, CPU budget, memory budget을 다른 한계가 여유인 상태에서 각각 포화시키는 case로 재매핑했다. B20의 primary case는 `local_runtime` 1/1 포화, exact capability blocker, 다른 physical-domain 무변경, unrelated blocking 진전을 검사한다. 같은 파일의 large-stack case는 role pool과 wider dedicated domain의 분리를 별도로 보강한다.
- H07은 `c238a63`에서 새 `hardening_policy_interactions::primary_scavenger_memory_fallback_and_drop_best_effort_keep_their_contracts`로 재매핑됐다. 이 커밋된 case는 primary, scavenger, memory fallback, explicit drop을 한 이력에서 검사한다. fallback class와 blocker 재평가, tier 순서, 최종 conservation assertion은 있다. 실행 결과는 최종 HEAD receipt에서 판정한다.
- H32는 다른 class의 measured overcommit, 대상 class holder release 뒤 memory-primary queue 유지, pressure release 뒤 promotion을 한 이력에서 직접 검증한다.
- D01은 disabled quota와 class 1/+1, queue depth 0/1/+1, pool 0/1/+1, CPU·memory global budget 각각 0/1/+1 및 과대 per-request cost를 명시적으로 구분한다. 경계 의미는 각 capacity kind별 대표 경계이며 모든 필드의 Cartesian 곱은 검증하지 않는다.
- 위 항목은 최초 매핑에서 발견한 주장 범위 차이를 기존 정확 fixture 연결, 최소 결합 fixture, 또는 명시적 범위 축소로 정리한 결과다. 이 감사에서 추가 production 결함은 확인되지 않았다.

## 인계 및 중단 조건

- Do not add a second executor queue or engine-specific pools.
- Changes to Governor/state serialize before BG25-009 and BG25-010.
