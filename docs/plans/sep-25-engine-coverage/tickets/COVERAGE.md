# 53개 공백의 단일 주 담당

원본 [104개 시나리오와 정적 K/P/G 감사](../../../misc/tmp-engine-checklist-sep-25.md)를 기준으로 한다. 각 P/G ID는 아래 한 티켓에만 배정한다. 티켓별 acceptance ID가 개별 판정 oracle이다. 교차 fixture는 다른 티켓의 증거를 재사용할 수 있어도 주 담당을 중복시키지 않는다.

| 티켓 | ID | 개수 |
|---|---|---:|
| [S25-002](S25-002-strict-ingress.md) | D04, D21, D22, D25, H33 | 5 |
| [S25-003](S25-003-wire-and-input-boundaries.md) | B18, B24, D03, D13, D14, D17, D23 | 7 |
| [S25-004](S25-004-identity-and-plan-scope.md) | B21, B22, B28, H18 | 4 |
| [S25-005](S25-005-executor-authority.md) | B25, H15, H27, H30, D16 | 5 |
| [S25-006](S25-006-deadline-and-custody.md) | D05, D15, H11, H12, H19, H20, H31, H34 | 8 |
| [S25-007](S25-007-root-child-boundary.md) | A11, H21, H35 | 3 |
| [S25-008](S25-008-admission-capacity.md) | A05, B05, B20, H01, H02, H07, H32, D01 | 8 |
| [S25-009](S25-009-queue-races.md) | B23, B27, H04, H06, H13, H14, H16, H25, D19 | 9 |
| [S25-010](S25-010-memory-ledger.md) | H10, D20 | 2 |
| [S25-011](S25-011-load-evidence.md) | H17, H28 | 2 |
| **합계** | A 2 + B 10 + H 26 + D 15 = **53** | **53** |

S25-001은 선행 계약, S25-012는 통합 증거를 소유하므로 시나리오 주 담당이 없다. 이미 K인 51개는 baseline regression으로 보존한다. `plan.json`과 `validate_plan.py`는 이 표·티켓·원본 감사의 ID 집합/의존성을 대조한다.
