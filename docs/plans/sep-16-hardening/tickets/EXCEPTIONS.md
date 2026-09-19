# 예외 대장 — 미체크 acceptance 항목의 owner·사유·재검토 조건

각 티켓의 "인계 / 완료 증거" 세 번째 항목("남은 예외는 owner·사유·만료/재검토 조건을 기록한다")의
단일 기록처다. 아래 항목만이 22개 티켓의 acceptance 중 `[ ]`로 남아 있으며, 각각은 **구현 누락이
아니라 기록된 결정·환경 제약·외부 의존**이다. 이 표에 없는 `[ ]`는 없어야 한다 (README의 감사 절차).

| 항목 | 분류 | 사유 | owner | 재검토 조건 |
|---|---|---|---|---|
| H16-006-A05 retained count/bytes 상한 | N/A (해당 구성요소 없음) | retirement worker를 두지 않았다. slow callback은 호출 스레드를 점유할 뿐 engine state를 잠그지 않는다 (ADR 0003 D07 effects-outside-lock) | 저장소 owner | retirement worker/비동기 effect queue를 도입하는 설계가 생기면 상한과 test를 같이 넣는다 |
| H16-009 metadata byte budget / declared payload bound | 결정 (범위 밖) | 제한 대상은 runtime-owned outstanding work다; caller가 만든 임의 heap의 hard-cap을 약속하지 않는다 (D01) | 저장소 owner | D01을 바꾸는 계약 변경이 있을 때 |
| H16-013-A03 diagnostics ring overflow | N/A (해당 구성요소 없음) | diagnostics ring을 두지 않았다 — 넘칠 버퍼가 없다 (`diagnostics_dropped` 필드도 제거) | 저장소 owner | diagnostics 버퍼를 도입할 때 |
| H16-015-A05 Linux instruction-count receipt | BLOCKED (환경) | `bench-iai`는 Linux/valgrind 전용; macOS local receipt는 SKIPPED_PLATFORM | 저장소 owner (push) | CI `qualification`·`bench.yml`이 Linux에서 돈 뒤 — 첫 run BASELINE_CREATED, 두 번째 run QUALIFIED |
| H16-017-A05 Linux >5% injection FAIL / control PASS | BLOCKED (환경) | 위와 같음; injection fixture는 `tools/bench/tests/test_iai_gate.py`에 있고 실제 valgrind run만 Linux | 저장소 owner (push) | 위와 같음 |
| H16-022-A04 Linux IAI qualification | BLOCKED (환경) | 이 세션의 credential은 `josongmin/quanta-taskmesh`에 403 — push 불가 | 저장소 owner (push) | push 후 CI `qualification` job의 receipt artifact |
| H16-022-A05 review/merge/consumer/activation | EXTERNAL (UNVERIFIED) | commit은 branch `hardening/sep-16`에만; review·merge·외부 consumer·activation은 이 저장소 밖 | 저장소 owner | PR review/merge 및 consumer repo+SHA 확인 시 |
| H16-022-A06 rollback 계약 — owner 승인 | EXTERNAL (문서화는 완료) | rollback 계약은 `docs/release-checklist.md`에 기록; 승인은 owner 행위 | 저장소 owner | 0.2.0 release 승인 시 |

## 인계 항목의 처리 (모든 티켓 공통 3줄)

1. **acceptance ID별 exact-source receipt·정상/negative 결과** — `docs/plans/sep-16-hardening/receipts/local-2026-09-19.json`
   (+ `.gates.json`, `.mutations.json`): clean committed tree(`cc5b256`)의 source digest(전후 일치)·24 gate 결과·mutation
   100 entry의 kill/control 결과(각 entry의 기대 실패 이유 포함). acceptance ID ↔ test 이름은 각 티켓의
   "검증 / 완료 조건"에, test ↔ finding은 `COVERAGE.md`와 `tools/verification/mutations.json`의 `finding`에 있다.
2. **lease 인계·production 통합·외부 소비자·activation 상태** — 단일 작업자라 lease 인계는 없었다.
   production 통합/외부 소비자/activation은 **UNVERIFIED**로 위 표(H16-022-A05)에 독립 표시했다.
3. **남은 예외** — 이 문서.
