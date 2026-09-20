# 최종 다각도 audit

## 증거와 범위

기준 HEAD는 README와 같다. 이번 작업은 기존 40개 finding 및 품질 메모를 current source/manifest/Justfile/workflow와 대조하고, engine/runtime와 tools/proof 두 독립 reviewer가 설계 누락을 검토한 정적 audit다. 새 production 코드나 테스트를 실행하여 전체 repo를 재검증한 것은 아니다.

원본 [audit 범위](../../../bugbash/sep-16-general/tickets/QUALITY-AND-SCOPE.md)와 [재현 증거](../../../bugbash/sep-16-general/tickets/evidence/verification.md)는 보존한다. 이전 runtime 20 / benchmark 6 관찰의 debug/release PASS는 역사적 결함 재현 성공이지 수정 완료가 아니다. TM16-015 interleaving은 정적 근거이며 TM16-005는 계약 결정이다. consumer MSRV/현재 hosted checks/배포 상태는 검증되지 않았다.

## 기존 제안에 보완한 설계 누락

| 관점 | 누락 시 실패 | 계획에서 강제한 보완 |
|---|---|---|
| ingress | bounded queue 앞 waiter/metadata가 무한 증가 | first-poll credit, whole-outstanding·metadata·terminal retention bound — H16-003/009 |
| 외부 executor | 여러 runtime capacity 이중 권위; accepted 뒤 환급하면 중복 실행 | shared reservation authority, accept/start/terminate, unknown outcome quarantine — H16-010/011 |
| effect 순서 | cancel 뒤 지연 dispatch가 실행; 완료 event가 full queue에서 유실 | generation/start authorization, reserved control return, continuation — H16-006/010 |
| cleanup | caller timeout을 worker dead로 취급; never-ending cleanup 무제한 | orthogonal response, cleanup charge/bound, NotDrained — H16-012 |
| deadline/clock | inline spawn/lock wait 누락; clock under-lock reentry | worker start timestamp, last-chance checks, out-lock custom clock — H16-005/012 |
| accounting | bytes→unit saturation, 역순 측정·중복 stage delta, overage 은폐 | checked conversion, exact oracle, epoch/idempotency, pressure debt — H16-004/005 |
| fairness | scheduler와 physical dispatch 이중 queue, idle credit 축적 | eligibility 합성, debit/rollback, explicit FIFO/HOL, operation budget — H16-007/010 |
| public compatibility | centralization이 !Send/borrowed/direct governor/custom adapter를 깨뜨림 | consumer inventory, ownership capability, additive migration — H16-001/008/011 |
| nested work | parent/child wait cycle을 boundedness로 해결했다고 오인 | 지원 제한 명시·선언된 cycle reject; 별도 DAG 엔진 제외 — D12 |
| gate parity | local-only/CI-only 검사 양쪽 누락; inventory 삭제도 green | 독립 required set, 양방향 parity, real fixture collection — H16-018 |
| negative proof | original test는 무력한데 별도 toy repro만 red | repaired original + same mutation only + positive control — H16-014/017 |
| baseline | fetch 실패를 '없음'으로 처리; schema 바뀐 수치 비교 | typed fetch status, compatible fingerprint, NOT_QUALIFIED bootstrap — H16-017 |
| MSRV/advisory | dev graph와 consumer graph 혼동; advisory 범위 과장 | workspace 밖 consumer fixture, 실제 toolchain, revision/time receipt — H16-019 |
| PM | duplicate YAML key·basename collapse·lint가 다른 파일 검증 | strict loader, exact render plan, nonmutating lint — H16-020 |
| proof custody | dirty source 이동/편집복구를 head만으로 감춤 | content/mode/symlink/untracked digest + isolated hosted checkout의 Actions/workspace/GITHUB_SHA 검증 — H16-022 |

이 표는 독립적으로 재현된 신규 결함 15개라는 뜻이 아니다. 기존 finding의 구조적 해결을 망가뜨릴 수 있는 설계 위험을 acceptance로 전환한 것이다. 특히 arbitrary nested wait cycle은 이번에 새 runtime bug로 확정하지 않았다.

## 과설계 방지

- mutex를 actor/lock-free로 바꾸는 대규모 rewrite가 필요하다는 근거 없음. narrow transition + bounded effects부터 시행한다.
- 제품/engine별 pool 증식 금지. capability pool과 class policy의 독립 의미 유지.
- 전체 오류를 로그로 바꾸거나 모든 unwrap/expect를 삭제하는 기계적 정리 금지. reachable failure와 ownership 영향을 기준으로 판단.
- unknown serde field, reserved enum/extension API, local affinity는 자동 결함이 아니다. [품질 disposition](COVERAGE.md)에서 유지/승인/수정 구분.
- model checker는 production seam에 연결된 범위만 입증한다. [Loom 공식 문서](https://docs.rs/loom/latest/loom/)의 synchronization replacement 조건에 맞추며 toy model 결과를 전체 runtime 보장으로 확대하지 않는다.

## 최종 판단

우선순위는 admission·lease lifecycle·capacity accounting·cleanup correctness이며, benchmark/gate 신뢰성은 독립 병렬 rail로 복구해야 한다. 코드 수정 전에 결정해야 할 계약은 12개다. 구현과 검증 모두 완료되지 않았으므로 전체 repo가 안전하다거나 SOTA 성능이라는 결론은 내리지 않는다.
