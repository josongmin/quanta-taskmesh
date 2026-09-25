# 계약 결정안 — 구현 전 고정

아래는 **권고안**이다. 현행 보장으로 승격하지 않는다. 각 결정은 S25-001에서 소스/소비자 영향과 함께 확정하고 library spec·external interface·해당 regression에 반영한다.

| 결정 | 권고안과 이유 | 호환성·실패 경계 |
|---|---|---|
| D-S25-01: untrusted JSON | 원시 Serde DTO와 별도인 **명시적 strict ingress**를 계약 crate 또는 공식 adapter에 둔다. byte cap은 deserialize **전**, 중첩 depth·stage count·중복/unknown key는 decode 동안, 의미 검증은 decode 뒤에 적용한다. 한계 수치는 측정된 사용 사례와 자원 budget으로 결정한다. | raw DTO의 permissive wire를 무심코 breaking 변경하지 않는다. strict 경로를 통과하지 않은 배포 설정/작업 입력을 안전하다고 부르지 않는다. 외부 ingress라면 owner와 통합 receipt가 필요하다. |
| D-S25-02: awaited child | untrusted child JSON에는 `parent_awaits`를 **명시적으로** 요구한다. false도 명시해야 한다. 내부 Rust builder의 `child_of`/`awaited_child_of`는 유지한다. | 과거 JSON의 누락→false 허용은 strict ingress에서 reject한다. 엔진은 opaque closure 내부 await를 추론할 수 없다. |
| D-S25-03: dispatch-control field | strict ingress에는 `shared_blocking` / `requested_stack { stack_size_bytes }`처럼 **명시적 dispatch 선택**을 둔다. requested-stack의 stack 누락·오타는 reject하고 선택된 role/physical pool을 validated plan에서 고정한다. | 기존 raw `TaskSpec`의 optional stack과 JSON roundtrip은 별도 계약. shared blocking은 유효한 명시적 선택이다. 기존 raw JSON에 새 tag를 요구하려면 versioned ingress/migration이 필요하다. |
| D-S25-04: parent stage membership | 현재 engine은 문자열과 live parent generation을 검증하지만 타 요청의 plan 전체 membership registry는 없다. cross-plan membership은 **외부 planner/조립자**의 명시적 책임으로 두고 engine이 지원하는 ancestor/cycle 경계만 보장한다. | membership을 engine hard guarantee로 요구한다면 parent plan 보관·generation·retention·API 변경을 별도 설계해야 한다. 단순 `MalformedTask` 기대를 추가하지 않는다. |
| D-S25-05: raw permit/ticket ID | 최종 목표는 Governor instance에 결속된 opaque handle이다. `PermitId`/`Ticket`의 현재 u64 alias는 서로 다른 Governor에서 충돌하므로 migration/semver를 먼저 평가한다. | breaking 이행 전에는 direct raw API를 **same-Governor scoped**로 문서화하고 foreign ID 안전성을 주장하지 않는다. 숫자 충돌 negative fixture를 유지한다. 문자열/숫자만으로 capability authority를 위조할 수 없다는 다른 계약과 혼동하지 않는다. |
| D-S25-06: deadline·정상 결과 | 현행 Accepted ADR의 `RunFor`는 **worker-start 기준 실행 예산**으로 유지한다. root가 예산 안에 끝나고 cleanup만 길면 늦게 성공을 전달할 수 있으며, 이를 `DeadlineExceeded`로 조용히 재분류하지 않는다. `CompleteBy`는 absolute admission-to-completion 계약에서 **caller 응답까지** 포함하도록 명시하는 안을 제안한다. cleanup이 그 시각을 넘기면 기한 내 terminal 응답을 보내고 lease는 child 종료까지 남긴다. 정상 성공을 관측한 caller에게는 release fence 완료를 유지한다. | `CompleteBy`의 응답 포함은 S25-001의 명시적 계약 결정이다. 기존 소비자가 다른 의미를 기대하면 migration 또는 별도 response-deadline 옵션을 설계한다. OS scheduler hard real-time 보장은 아니며 `shutdown_timeout`은 종료 증거가 아니다. |
| D-S25-07: Tokio 밖 poll | `run_blocking`/default CPU가 Tokio context를 필요로 하는지 공개 계약으로 정한다. 지원하지 않는다면 host가 admission·worker 생성 **전** typed context error를 내고 상태를 바꾸지 않는 쪽을 권고한다. | context 없는 `spawn_blocking` panic은 문서상 허용된 graceful failure로 포장하지 않는다. Rayon CPU와 custom adapter의 context 요구는 따로 검증한다. |
| D-S25-08: 공유 executor | runtime별 Governor는 자기 제출만 제한한다. 여러 runtime의 합산 물리 상한은 공유 capacity authority가 **명시적으로 설치된 경우**에만 주장한다. | `exclusive_pool=false`의 ambient work는 관리 밖. engine별 새 pool을 늘려 숫자만 맞추지 않는다. |
| D-S25-09: wire 소비자 | `Snapshot.schema_version`은 Snapshot 전용이다. 미래 verdict/terminal enum variant는 구버전 derived Serde에서 decode 오류를 낸다. 소비자는 오류/version negotiation 경로를 가져야 한다. | 다른 타입까지 Snapshot version으로 호환성을 주장하지 않는다. 추가 tolerant envelope가 필요하면 별도 versioned wire 계약을 설계한다. |

외부 의존성의 현재 문서는 설계 근거일 뿐, 저장소 lock 버전의 실행 증거가 아니다. 특히 Tokio의 non-abortable blocking 작업과 Serde default/unknown-key 동작은 현재 소스 fixture로 최종 판정한다.
