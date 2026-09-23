# 예외 대장 — 미체크 acceptance 항목의 owner·사유·재검토 조건

각 티켓의 "인계 / 완료 증거" 세 번째 항목("남은 예외는 owner·사유·만료/재검토 조건을 기록한다")의
단일 기록처다. 아래 항목만이 22개 티켓의 acceptance 중 `[ ]`로 남아 있으며, 각각은 **구현 누락이
아니라 기록된 결정·환경 제약·외부 의존**이다. 이 표에 없는 `[ ]`는 없어야 한다 (README의 감사 절차).

| 항목 | 분류 | 사유 | owner | 재검토 조건 |
|---|---|---|---|---|
| H16-006-A05 retained count/bytes 상한 | N/A (해당 구성요소 없음) | retirement worker를 두지 않았다. slow callback은 호출 스레드를 점유할 뿐 engine state를 잠그지 않는다 (ADR 0003 D07 effects-outside-lock) | 저장소 owner | retirement worker/비동기 effect queue를 도입하는 설계가 생기면 상한과 test를 같이 넣는다 |
| H16-009 metadata byte budget / declared payload bound | 결정 (범위 밖) | 제한 대상은 runtime-owned outstanding work다; caller가 만든 임의 heap의 hard-cap을 약속하지 않는다 (D01) | 저장소 owner | D01을 바꾸는 계약 변경이 있을 때 |
| H16-013-A03 diagnostics ring overflow | N/A (해당 구성요소 없음) | diagnostics ring을 두지 않았다 — 넘칠 버퍼가 없다 (`diagnostics_dropped` 필드도 제거) | 저장소 owner | diagnostics 버퍼를 도입할 때 |
| H16-015-A05 Linux instruction-count receipt | BLOCKED (환경) | `bench-iai`는 Linux/valgrind 전용; macOS 결과는 Linux proof가 아니다 | Linux qualification owner | clean Linux 후보에서 동일 fingerprint로 baseline 생성 후 `QUALIFIED` run의 raw evidence 확보 |
| H16-017-A05 Linux >5% injection FAIL / control PASS | BLOCKED (환경) | `tools/bench/tests/test_iai_gate.py`의 fixture는 있으나 실제 Linux valgrind run의 source-bound 결과가 없다 | Linux qualification owner | clean Linux 후보에서 control 및 >5% injection 결과 기록 |
| H16-022-A04 Linux IAI qualification | BLOCKED (증거) | 현재 GitHub verification workflows는 수동 비활성화다; 과거 credential 403은 현재 qualification 경로의 blocker가 아니다. 현 HEAD의 local Linux ordinary receipt가 없다. local collector는 시작·종료 digest를 확인하지만 중간 edit-and-restore를 배제하지 않는다 | Linux qualification owner | writer 없는 배타적 clean Linux checkout에서 `just qualify-local` 후 `just validate-local-qualification`이 같은 SHA에 `QUALIFIED` |
| H16-022-A05 review/merge/consumer/activation | EXTERNAL (부분 미검증) | hardening 구현은 `origin/main@e55d3aa`에 포함됐다. 2026-09-23 로컬 `semantica-codegraph-v2@f2b5180b28e55d8fc268ec6fb299f1d16a81995b`의 추적된 `quanta-runtime/Cargo.toml`에 Taskmesh 0.3.0 optional path dependency와 `taskmesh-governance-test-hooks` 소비자 테스트가 확인됐다. 해당 checkout은 dirty이며 이 정확한 두 소스의 consumer compile/test, PR review, deployment 및 activation 증거는 없다 | release/consumer owner | Semantica의 frozen SHA/source와 Taskmesh 후보를 결합한 소비자 rail, review disposition, deployment·activation identity를 각각 확인 |
| H16-022-A06 rollback 계약 — owner 승인 | EXTERNAL (문서화는 완료) | rollback 계약은 `docs/release-checklist.md`에 기록; 승인은 owner 행위 | 저장소 owner | 현재 `0.3.0` 후보의 release 결정 및 rollback 승인 시 (`tools/release/release-policy.json`의 version decision은 PENDING) |

## 인계 항목의 처리 (모든 티켓 공통 3줄)

1. **acceptance ID별 역사적 local 증거** — `docs/plans/sep-16-hardening/receipts/local-2026-09-19.json`
   (+ `.gates.json`, `.mutations.json`): clean committed tree(`cc5b256`)의 source digest(전후 일치)·24 gate 결과·mutation
   100 entry의 kill/control 결과(각 entry의 기대 실패 이유 포함). 이 bundle은 `NOT_QUALIFIED`이며 현재
   source의 qualification receipt가 아니다. acceptance ID ↔ test 이름은 각 티켓의
   "검증 / 완료 조건"에, test ↔ finding은 `COVERAGE.md`와 `tools/verification/mutations.json`의 `finding`에 있다.
2. **lease 인계·production 통합·외부 소비자·activation 상태** — 당시 단일 작업자라 lease 인계는 없었다.
   hardening commit의 `origin/main` 포함은 현재 확인했다. Semantica의 추적된 의존 경로는
   발견했지만 소비자 실행/activation은 **UNVERIFIED**로 위 표(H16-022-A05)에 독립 표시했다.
3. **남은 예외** — 이 문서.
