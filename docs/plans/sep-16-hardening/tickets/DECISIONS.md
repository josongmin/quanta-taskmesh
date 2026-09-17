# 계약 결정 — D01–D12

모든 항목 상태는 **ACCEPTED (구현됨)** 이다. 선택·근거·거부한 대안·소비자 영향은
[ADR 0003](../../../adr/0003-sep-16-hardening-contracts.md)에 기록했고, 각 계약에는
`crates/**/tests/hardening_*.rs`의 regression이 붙어 있다.

아래 표의 "권고" 열은 계획 당시의 제안이며, 실제로 채택된 계약은 ADR 0003이 기준이다.
제품 owner의 외부 승인 기록은 여전히 별도이며 이 문서가 대신하지 않는다.

| ID | 책임 역할 | 권고 | 영향 티켓 | 승인해야 할 경계 |
|---|---|---|---|---|
| D01 | I/E | first-poll에 owned admission; positive global/class/capability outstanding limits. strict profile에서 0=unbounded 금지. opaque caller closure heap은 bounded라고 주장하지 않음 | [H16-002](H16-002-validated-policy-topology.md), [H16-003](H16-003-terminal-ticket-lifecycle.md), [H16-009](H16-009-bounded-intake.md) | unpolled future는 caller 소유. polling/credit 획득 후만 internal request cell 생성; legacy unbounded 설정 migration 및 overflow 결과 승인 |
| D02 | E | stage release/measurement를 별개 epoch event로 정의; released reservation 부활 금지; strict runtime task는 stale heartbeat만으로 capacity 환급 금지 | [H16-004](H16-004-exact-resource-accounting.md), [H16-005](H16-005-memory-epochs-and-lease-clock.md) | TM16-005 early release와 OnTaskCompletion 계약, duplicate delta, Suspected→terminal 권한 확정 |
| D03 | I/R | resolved execution plan 고정; fallback은 기본적으로 resource routing만 바꾸고 declared class semantic policy/control identity 보존 | [H16-008](H16-008-resolved-execution-plan.md), [H16-010](H16-010-dispatch-integration.md) | 현재 fallback 기대와 다른 소비자가 있으면 full-policy fallback을 explicit variant로 제공하거나 breaking migration 승인 |
| D04 | I/R | requested stack이 실제 LargeStack capability를 소비; 일반 CPU/blocking pool limit로 우회 불가 | [H16-008](H16-008-resolved-execution-plan.md), [H16-011](H16-011-executor-protocol-and-setup.md) | stack hint/guarantee 구분, platform thread stack 범위, compatibility matrix 확정 |
| D05 | R/I | try-reserve→accept→start→terminate adapter protocol; runtime 여러 개가 공유하는 executor는 동일 capacity authority 사용 | [H16-009](H16-009-bounded-intake.md), [H16-010](H16-010-dispatch-integration.md), [H16-011](H16-011-executor-protocol-and-setup.md) | 외부 ambient tasks는 managed budget 바깥임을 명시. capability 없는 legacy adapter는 conservative one-slot 또는 strict reject 중 승인 |
| D06 | E/I | per-request checked byte/unit 변환; aggregate checked wide integer; snapshot wire는 versioned exact representation | [H16-004](H16-004-exact-resource-accounting.md), [H16-013](H16-013-observability-and-invariants.md) | u128/문자열 wire/기존 DTO adapter, overage debt 처리 및 public conversion overflow 오류 결정 |
| D07 | E | monotonic commit time, lease generation/measurement epoch. external Clock callback은 mutex 밖에서 호출하고 commit에서 monotonic clamp | [H16-005](H16-005-memory-epochs-and-lease-clock.md), [H16-012](H16-012-deadlines-and-cleanup.md) | clamp의 논리시각과 실제 elapsed time 차이, fake/backward clock contract, clock panic 분류 |
| D08 | F/I | physical+semantic eligible일 때만 fairness debit. dispatch commit에 debit, pre-start failure rollback. class 내부 strict FIFO를 기본 제안 | [H16-007](H16-007-fairness-reference-and-credit.md), [H16-010](H16-010-dispatch-integration.md) | eligible/blocked 클래스 정의, mixed capability HOL tradeoff, cancelled service debt와 retry hint 의미 |
| D09 | I/R | Acquire는 lock wait 포함; last-chance claim/start authorization에서 재검사. RunFor는 worker start 기준 | [H16-002](H16-002-validated-policy-topology.md), [H16-012](H16-012-deadlines-and-cleanup.md) | ZERO=즉시시도 별도 API 여부, completion/deadline tie, nonabortable blocking RunFor의 unsupported vs response-only variant 승인 |
| D10 | R/I | caller response와 task resource lifecycle 분리; worker/owned children/runtime teardown 완료가 release fence | [H16-011](H16-011-executor-protocol-and-setup.md), [H16-012](H16-012-deadlines-and-cleanup.md) | caller timeout은 강제종료 아님. never-ending cleanup charge 유지, bounded drain 후 NotDrained. detached unmanaged child는 지원 범위 밖 |
| D11 | Q/I | PM는 validated exact path render plan; lint read-only; actual target apply는 별도 승인 | [H16-020](H16-020-pm-validated-render-plan.md) | containment/symlink/duplicate YAML 정책, AGENTS 등 사용자 target migration 소유권 |
| D12 | I/R | 동일 capability parent-await-child 등 선언된 wait cycle은 지원 금지/사전 reject; opaque closure waitgraph 자동 감지는 범위 밖 | [H16-009](H16-009-bounded-intake.md), [H16-010](H16-010-dispatch-integration.md), [H16-012](H16-012-deadlines-and-cleanup.md) | borrowed IO·!Send local 보존; capacity 대여/structured DAG scheduler 신규 개발은 별도 RFC |

## 공통 migration 규칙

- raw public DTO를 곧바로 삭제하지 않고 validated internal type을 추가한다. default/Rayon/direct Governor/custom Clock/custom executor/borrowed IO/non-Send local 소비자를 inventory화한다.
- public field 비공개화, snapshot 정수 폭/serde 표현, claim 반환 타입, timeout/fallback 의미 변경은 additive adapter 또는 명시적 breaking release를 선택한다.
- 기존 profile의 0/unbounded 의미를 몰래 재해석하지 않는다. strict profile에 양수 bound를 요구하고 legacy 예외는 scope·owner·만료를 기록한다.
- capacity 수치·drain timeout·latency threshold는 제품 workload/실제 운영 owner가 승인한다. 이 문서는 근거 없는 production 기본값을 정하지 않는다.
- 제품별 class 이름은 neutral public API에 새 하드코딩하지 않는다. semantic policy와 capability topology registry를 독립 검증한 뒤 execution plan에서 합성한다.

