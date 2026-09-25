# Sep-25 병렬 잔여 작업 계획

최초 조사 기준: 2026-09-25의 clean `c87e0832db0ec5b29d52c05213e872c442cc7ea3`. 이 커밋의 macOS CI-profile 영수증은 16/16 PASS, `qualified=true`다. **이후 커밋 및 현재 미커밋 변경에는 해당 영수증을 재사용할 수 없다.** 착수·통합 직전에 HEAD, tree, dirty 경로와 각 변경의 소유자를 다시 고정한다.

BG25-001~012의 저장소 구현은 위 커밋에 들어갔다. 이 문서는 과거 구현 웨이브를 재실행하는 계획이 아니라, 남은 의미 검증·외부 채택·최종 exact-source 확인의 작업 배치다. `EXECUTION.md`의 W0~W4는 역사적 통합 기록으로 취급한다.

## 남은 작업의 종류

| 종류 | 현재 판정 | 완료 경계 |
|---|---|---|
| 저장소 구현 | BG25-001~012 구현됨. 이번 재감사에서 추가 production 결함은 아직 확정되지 않음 | 새로운 source 수정은 결정적 RED 반례가 있을 때만 |
| 시나리오 의미 검증 | 104/104 `MAPPED`는 테스트 이름이 있는 **정적 후보 매핑**. 실행·oracle 충족을 뜻하지 않음 | 각 주장과 실제 assertion을 대조하고 과장된 행은 좁히거나 최소 fixture를 추가 |
| 현재 dirty 변경 | 기존 `c87e083` receipt 밖 | 소유자별 focused proof → 직렬 통합 → clean final HEAD receipt |
| 배포 채택 | 외부 bytes ingress, parent planner, wire consumer 호출점·소유자 미확인 | 실제 consumer 변경과 그 저장소의 통합 증거. 이 저장소 테스트로 대체 불가 |
| nightly/release | 미실행 | 별도 명시적 승인 필요. 보통의 Mac-local 작업에 mutation/modelcheck/TSan/fuzz 캠페인 포함 금지 |

## 병렬 레인과 파일 소유권

| 레인 | 지금 할 일 | 단독 쓰기 범위 | 산출물 / DoD |
|---|---|---|---|
| A — 계약·소비자 | BG25-001~004의 D1~D9 승인 기록과 실제 외부 call site를 대조한다. P/G 16행(BG25-002~004)을 감사한다. strict `parse_task_spec`/`parse_runtime_config`, parent-stage membership, Snapshot 버전/미래 enum 처리, opaque-handle 소비자 마이그레이션을 각각 별도 상태로 둔다. | 외부 consumer는 해당 저장소 owner가 작업한다. 이 저장소에서는 공개 spec·계약 소스의 단일 owner만 수정한다. 현재 dirty BG25 티켓 문서는 소유권 확인 전 손대지 않는다. | 배포 ingress/planner/wire의 **정확한 저장소·호출점·담당자**, 호환성 판단, consumer test/receipt 또는 `EXTERNAL_OPEN`. 찾지 못했으면 구현 완료와 제품 채택 완료를 합치지 않는다. |
| B — host·lifetime | BG25-005~007/011의 P/G 18행과 BG25-008 중 host 경계를 감사한다. 우선 H01의 다중 class·IO/blocking/CPU/large-stack 과부하, H21/H35의 root/child 범위, H28의 실제 host와 simulator 비교 주장을 현 테스트와 대조한다. | `crates/taskmesh/src/{builder,runtime}.rs`, host 테스트는 host owner 1명. 현재 dirty `hardening_root_child_scope.rs`는 기존 작성자 소유. | 누락된 복합 주장에는 **하나의 경계가 명확한 대표 fixture** 또는 정직한 주장 축소. 응답·worker lease·drain을 별도 사건으로 계상. 새 source 패치는 RED 반례 뒤에만. |
| C — engine·oracle | BG25-008~010의 P/G 19행을 소유하고 H06/H07/H16/D01 등을 실제 fixture와 대조한다. H01 host fixture는 B에게 요구·인계한다. queue/memory/admission 기대값은 입력 이벤트로 독립 계산한다. | 별도 test/model 파일은 병렬 가능. `crates/taskmesh-engine/src/engine/{governor,state}.rs`는 owner 1명이 직렬 수정. | H06의 mixed WFQ/DRR·cross-pool, H07의 tier/scavenger/memory, H16의 진짜 두 스레드 interleaving, D01의 각 경계가 assertion으로 증명되거나 의미 검토 결과 `GAP`으로 남는다. 스냅샷 자기비교를 독립 oracle로 세지 않는다. |
| D — 증거·통합 | BG25-012가 기존 K 51행을 직접 감사하고 A~C의 P/G 53행 결과를 case 단위로 검수하며 Cargo collection/gate selector를 검증한다. 현재 dirty `validate_scenario_evidence.py`와 `tools/gates/tests/test_scenario_evidence.py`는 기존 작성자가 검증·인계한다. | integrator만 `scenario-evidence.json`, 검증기, `Justfile`, `tools/gates/**`, 최종 main/receipt를 통합한다. | 51 K + 34 P + 19 G의 분모 유지. `MAPPED`/실행 PASS/의미 충족을 분리해 기록. collected case·feature·platform·gate가 일치하고 최종 clean HEAD의 `verify-macos-ci`를 재검증. |

레인 A~C는 **읽기·테스트 설계·서로 다른 신규 fixture**를 병렬로 진행한다. 공유 production 파일과 최종 증거 스키마는 병렬 쓰기 금지. 각 작업자가 별도 worktree/branch와 `CARGO_TARGET_DIR`를 쓰되, Mac의 무거운 Cargo build는 동시에 돌리지 않는다. 주 작업트리의 미커밋 변경을 옮기거나 덮어쓰지 않는다.

### 우선 의미 검토 대상 (확정 버그 목록 아님)

| ID / owner | 현 매핑 | 우선 확인할 미증명 주장과 최소 처리 |
|---|---|---|
| H01 / B→C | `crates/taskmesh/tests/e2e_scenarios.rs::overload_is_bounded_and_fail_closed` | 현재 단일 class·IO 포화가 다중 class·네 substrate 과부하까지 증명하는지 확인. 인접 `e2e_chaos.rs`와 `host_open_loop.rs`를 재사용할 수 있는지 먼저 보고, 부족하면 유한한 mixed-substrate 대표 fixture 하나만 추가. |
| H06 / C | `crates/taskmesh-engine/tests/hardening_fairness_reference.rs::cancelling_from_the_middle_preserves_the_order_of_the_survivors` | 같은 class FIFO 취소와 mixed weight/cost WFQ·DRR, cross-pool follower 순서는 다른 주장. 기존 fairness fixture를 합쳐도 부족한 부분만 보강. |
| H07 / C | `crates/taskmesh-engine/tests/fairness_best_effort.rs::the_whole_primary_tier_precedes_the_whole_best_effort_tier` | tier 순서 외 scavenger·memory fallback까지 실제 assertion이 있는지 전역 테스트 검색 후 판단. |
| H16 / C | `crates/taskmesh-engine/tests/differential_model.rs::engine_agrees_with_the_reference_model` | 순차 differential model과 두 스레드 interleaving/replay를 분리. 동시 이력 증거가 없으면 별도 bounded fixture 또는 의미 검토 `GAP`. |
| H28 / B | `crates/taskmesh/tests/host_open_loop.rs::offered_terminal_unanswered_and_execution_counts_close_exactly` | 현재 작은 단일 class burst의 정확한 계수와 simulator와의 결과 범위 비교는 별개. 비교가 요구사항인지 결정하고 최소 측정 fixture를 추가하거나 과장된 oracle을 정정. |
| D01 / C | `crates/taskmesh-engine/tests/hardening_admission_matrix.rs::capacity_zero_one_exact_and_plus_one_are_unambiguous` | CPU budget 외 quota/depth/pool 각각의 0/1/exact/+1 근거를 확인. 이미 있는 경계 테스트를 연결하고 빈 경계만 추가. |

위 여섯 행은 빠른 탐색 출발점일 뿐이다. A/B/C/D는 각자 할당된 **전체 16/18/19/51행**을 감사한다. 새 검증기가 `#[ignore]` 또는 cfg 비수집 함수를 이름만으로 인정하지 않는지도 D가 실제 Cargo 수집과 대조한다.

## 직렬 통합 순서

1. **F0 — 소유권 고정:** 현재 dirty diff를 파일별 작성자와 묶고, peer가 진행 중인 변경이 완료될 때까지 겹치는 파일은 읽기 전용으로 둔다. 과거 receipt를 현재 상태에 재사용하지 않는다.
2. **F1 — 의미 감사 병렬:** A는 계약/consumer, B는 host, C는 engine, D는 case/selector를 나눠 104행을 검토한다. 각 행에 `주장 → 실제 assertion → 빠진 하위 주장 → 처리(충족/최소 fixture/주장 축소/외부 OPEN)`를 남긴다. 우선 H01, H06, H07, H16, H28, D01을 검사하되 이 여섯 개만으로 전체 104행을 완료했다고 하지 않는다.
3. **F2 — 최소 보완:** 중복 시나리오는 기존 fixture 한 개에 assertion을 보강한다. 새 테스트는 상호작용이나 경계가 실제로 빠졌을 때만 추가한다. 생산 코드 불일치가 재현되면 owner가 원인 경로를 수정하고 direct consumer까지 focused 검증한다. 시나리오 설명 자체가 제품 범위보다 넓으면 oracle/상태를 좁혀 과장 없이 남긴다.
4. **F3 — 증거 통합:** D가 변경된 test case를 실제 Cargo 수집 결과와 gate 선택기에 연결한다. 정적 검증기 PASS만으로 의미 충족이나 실행 PASS를 선언하지 않는다. 외부 caller가 없는 주장은 별도 adoption ledger에 `EXTERNAL_OPEN`으로 남긴다. 기존 manifest의 `MAPPED`는 정적 후보 의미를 유지한다.
5. **F4 — 단일 최종 검증:** 모든 쓰기가 멈춘 뒤 통합자는 focused checks, `just dev`, clean unchanged-HEAD `just verify-macos-ci`를 Mac 로컬에서 한 번 수행한다. receipt의 HEAD/tree/path digest, 16 CI-profile gate의 PASS/FAIL/NOT_RUN, skipped/exclusions를 검증한다. 변경이 다시 생기면 영수증을 재발행한다. GitHub Actions는 쓰지 않는다.

## 레인 인계 형식과 중단 조건

각 레인은 `시나리오/티켓 ID`, `base HEAD/tree`, `변경 경로`, `실제 assertion과 독립 기대값`, `focused 명령·결과`, `남은 외부 소유자/결정`을 제출한다. 다음 중 하나면 통합하지 않는다: 다른 레인 파일을 무단 수정, 미수집 test 이름, 현재 소스와 다른 SHA의 PASS, production snapshot을 기대값으로 재사용, 비용 높은 캠페인의 부분 결과를 자격으로 주장, 배포 consumer 미확인을 라이브러리 테스트로 폐쇄.

완료 보고는 세 줄로 분리한다: **(1) 저장소 코드/테스트**, **(2) exact-source Mac CI-profile**, **(3) 외부 채택 및 nightly/release**. 뒤의 두 항목이 열려 있으면 전체 완료율을 100%로 표기하지 않는다.
