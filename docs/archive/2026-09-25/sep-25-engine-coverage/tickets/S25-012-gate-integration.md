# S25-012 — scenario 증거·feature·CI 통합

- 상태: PLANNED; 우선: P1; 선행: [S25-002](S25-002-strict-ingress.md)–[S25-011](S25-011-load-evidence.md); 소유: 통합/CI owner.
- 근거: `Justfile`, `tools/gates/{inventory,required}.json`, `docs/plans/2026-09-24-ci-verification-stages.md`, `.github/workflows/ci.yml`, [release checklist](../../../../release-checklist.md).

## 목적

53개 P/G 시나리오의 핵심 oracle이 실제 gate에서 실행되는지 입증한다. 기존 `tools/gates/target_catalog.py`는 Cargo/Python **target→gate** census를 소유한다. 이 티켓은 scenario→핵심 target/case→gate→source-bound receipt의 연결만 추가하고 모든 test function을 복제한 두 번째 inventory를 만들지 않는다. 현재 `just test-rayon`은 host library와 integration **한 case**만 선택한다. `.github/workflows/ci.yml`은 manual release형 workflow이므로 이를 PR에 바로 연결하면 mutation/fuzz까지 자동 실행된다.

## 변경 파일

| 경로 | 변경 목적 | 적용 조건 |
|---|---|---|
| `docs/archive/2026-09-25/sep-25-engine-coverage/tickets/plan.json`, `validate_plan.py`, `COVERAGE.md`, `VERIFICATION.md` | 시나리오 주 담당·acceptance·실제 fixture/rail 연결을 검증 | 항상; 이 티켓의 문서 owner |
| 제안 `docs/plans/sep-25-engine-coverage/tickets/scenario_evidence.json` | 53개 ID별 **핵심** target/case/oracle/gate/feature·platform/status와 source를 기록 | 실제 fixture가 만들어진 뒤 추가. 테스트 전체 목록을 복제하지 않음 |
| `tools/qualification/tests/test_plan_bookkeeping.py` 또는 새 owner test | Sep-25 validator가 `py-test`에서 자동 실행되고 누락/중복/허위 fixture negative case가 실패 | plan을 tracked artifact로 통합할 때 |
| `Justfile`의 `test-rayon` | 새 Rayon feature host fixture 선택 | Rayon 대상이 실제로 추가된 경우만 |
| `tools/gates/target_catalog.py`, `tools/gates/tests/test_inventory.py` | 새 selector의 target census 및 빠진 selector negative proof | recipe/target 선택이 바뀐 경우만 |
| `tools/gates/inventory.json`, `tools/gates/required.json`, `tools/gates/validate_inventory.py`, `.github/workflows/ci.yml` 또는 신규 PR workflow | gate 의미/required membership/hosted trigger 변경의 독립 parity | 필요할 때만. 같은 리뷰에서 required 집합과 workflow 정책 대조 |
| `docs/misc/tmp-engine-checklist-sep-25.md`, `docs/release-checklist.md` | P/G 상태와 최종 증거 경계 갱신 | 정확한 fixture·receipt가 있는 경우만 |

`tools/gates/inventory.json`과 `required.json`은 서로 독립된 권위다. selector만 늘었다면 두 목록을 불필요하게 늘리지 않는다. `tools/qualification`·CI 파일은 integration owner가 단일 적용한다.

## 구현 순서

1. S25-002–011 인계에서 scenario ID, fixture path/test case, 정상·negative oracle, feature/platform, 실행 gate, source identity를 수집한다. 현재 51개 K는 baseline 회귀로 보존한다.
2. `scenario_evidence.json`의 53개 행을 **실제 존재하는 fixture**만으로 채운다. target은 Cargo metadata/기존 `target_catalog.py`와 교차하고 case는 실제 collection 결과에서 존재·실행·PASS를 확인한다. 미구현 항목은 OPEN/NOT_RUN으로 남긴다.
3. validator에 ID 중복·누락, P/G↔티켓 mapping drift, 존재하지 않는 target/case, feature/OS skip, disabled/ignored/zero-test 선택, gate 누락을 거절하는 규칙과 negative fixture를 추가한다. `py-test`가 실제 validator를 실행하도록 등록한다.
4. `just test-rayon`의 선택 범위를 변경했다면 `target_catalog.py`의 selector 검사와 실제 collection을 같이 갱신한다. adapter의 기본 workspace test와 host feature bridge는 별개다.
5. clean/exclusive checkout에서 `just verify-macos-ci`로 16개 CI-profile required gate를 실행한다. `target/verification/macos-gates.json`의 HEAD/tree/path digest와 required not-run/skip을 판정한다. owner-local PASS를 이 receipt로 승격하지 않는다.
6. 자동 PR 보호가 별도 요구되면 `docs/plans/2026-09-24-ci-verification-stages.md` W4에 맞춰 **CI-profile 전용** workflow를 만들고 inventory validator의 수동 전용 정책을 승인된 변경으로 갱신한다. required status check는 최신 commit SHA와 실제 branch/ruleset 설정에서 확인한다. merge queue를 사용한다면 `merge_group`도 포함한다. [GitHub status checks](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).

## DoD

- [ ] `S25-012-A01` 53개 P/G ID마다 핵심 oracle, 실제 target/case, executing gate, source가 연결된다. K/P/G를 증거에 맞춰 갱신하고 기존 K 51개를 삭제하지 않는다. **coverage K와 목표 계약 해결은 별도 상태**로 보관한다.
- [ ] `S25-012-A02` default/Rayon/consumer/MSRV/doctest/rustdoc/bench smoke 중 변경이 닿는 surface를 실제 command로 선택한다. target/case 없는 PASS, ignored/conditional skip, zero-test, Rayon selector 누락은 실패한다.
- [ ] `S25-012-A03` inventory와 required 독립 목록, Justfile expansion, workflow event/command parity가 일치한다. 자동 PR 보호는 전용 bounded workflow와 실제 branch/ruleset check가 관측된 경우에만 주장한다.
- [ ] `S25-012-A04` owner-local 결과와 clean exact-HEAD CI-profile receipt를 분리 기록한다. receipt의 source fingerprint/required denominator와 실제 artifact를 검증한다. stale/partial/nightly subset/coverage 숫자로 qualification을 대체하지 않는다.
- [ ] `S25-012-A05` release qualification이 명시적으로 요청될 때만 mutation 포함 nightly/release 완전 집합과 Linux-only/conditional rail을 수행한다. macOS platform-scoped receipt를 Linux release proof로 승격하지 않는다.
- [ ] `S25-012-A06` plan validator가 `py-test`에 등록되고, ID 누락·허위 fixture·feature selector 삭제의 negative test가 각각 예상 이유로 실패한다.

## 계획된 검증

`python3 docs/archive/2026-09-25/sep-25-engine-coverage/tickets/validate_plan.py`는 문서·매핑만 검증한다. 구현 시 `just dev-python-lint tools/gates tools/qualification`, `just dev-python-tests tools/gates/tests tools/qualification/tests`, `just gates-inventory`, `just test-rayon`, `just test`, `just consumer-msrv`, `just verify-macos-ci`를 변경 범위에 맞춰 실행한다. 이 목록은 **실행 계획**이며 이번 문서 작업의 결과가 아니다.

## 인계·중단 조건

fixture가 존재해도 gate가 선택하지 않거나 source가 dirty/변경되면 통합 완료를 선언하지 않는다. hosted workflow나 branch protection 변경은 현재 manual-only 정책과 충돌하므로 별도 승인·parity 리뷰 없이 적용하지 않는다. 외부 ingress/consumer의 실제 적용이 확인되지 않으면 해당 ticket의 통합 DoD는 OPEN으로 남긴다.

현재 요청은 계획 문서 작성이다. 이 티켓 작성 자체는 테스트·CI·mutation 실행 권한이나 합격 영수증이 아니다.
