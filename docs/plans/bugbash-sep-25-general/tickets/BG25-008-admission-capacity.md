# BG25-008 — compound admission과 capacity authority

- 상태: IMPLEMENTED
- 우선순위: P1
- 선행: BG25-001, BG25-005
- 소유: engine/admission owner; host capacity fixture는 host owner

## 목적

class, capability, CPU, measured memory, tier/fallback이 동시에 충돌할 때 원자성, blocker priority, boundedness를 independent event ledger로 증명한다.

## 근거

- 현재 intake는 모든 blocker를 계산하고 primary ordering을 적용한다.
- exact accounting과 single-dimension fixtures는 강하다.
- 여러 class/resource가 겹치는 direct oracle과 real-host open-loop evidence는 부족하다.
- engine audit에서 새 source defect는 확인되지 않았으므로 counterexample 우선이다.

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
- 이 oracle들에서 추가 production mismatch는 관측되지 않았다. 최종 자격은 BG25-012의 clean exact-source receipt에 종속된다.

## 인계 및 중단 조건

- Do not add a second executor queue or engine-specific pools.
- Changes to Governor/state serialize before BG25-009 and BG25-010.
