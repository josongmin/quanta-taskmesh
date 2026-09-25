# 증거·gate·종료 계약

## 시나리오 판정

| 상태 | 인정 기준 |
|---|---|
| K | 핵심 반례가 공개/production entrypoint에서 직접 실행되고, 필요한 값·부작용·독립 ledger를 assertion으로 판정한다. 해당 fixture가 실제 gate에 선택된다. |
| P | 관련 fixture는 있으나 조합/독립 oracle/consumer 경계 중 일부가 없다. |
| G | 직접 fixture가 없거나 선행 계약 결정이 없다. |

K는 해당 행의 모든 직교 조합, clean-head CI PASS 또는 release qualification을 뜻하지 않는다. 현재 51/34/19는 **정적 감사** 결과다. 상태 갱신에는 fixture path, test name, 선택 gate, source digest, 기대 실패 이유를 함께 기록한다.

**coverage와 해결은 별개 축이다.** 현행 permissive 동작을 정확히 고정한 fixture로 G→K가 되어도 strict ingress/opaque handle/응답 계약의 목표 구현이 남으면 티켓은 OPEN이다. 반대로 코드만 수정하고 반례 fixture·gate 선택이 없으면 coverage는 G/P로 남는다. 티켓 종료에는 두 축 모두의 근거가 필요하다.

## fixture 설계

1. 기능 단위에는 정상 control과 해당 결함을 드러내는 negative input을 쌍으로 둔다. 부작용 0은 worker call, permit/ticket, queue, pool occupancy를 확인한다.
2. 경합은 sleep만으로 일정을 추정하지 않고 barrier/주입 clock으로 전이점을 고정한다. hang oracle은 종료 상한이 있는 격리 프로세스 또는 명시적 timeout 계약으로 둔다.
3. 원장 oracle은 입력 event에서 별도로 계산한다. engine snapshot, `conservation_violation()`, 같은 helper의 두 결과를 서로 비교해 독립 proof라 부르지 않는다.
4. Loom/Shuttle은 production `sync` seam을 쓰는 좁은 상태 기계만 확인한다. replay seed와 explored bound를 기록한다. TSan은 실제 메모리 경합 rail이며 모델 대체가 아니다.
5. 실측 bench는 source, toolchain, feature, machine, quiet-host, workload/seed, warmup, sample denominator, baseline을 기록한다. simulator admission wait와 host latency는 별도 metric이다.

## rail 매핑

| Rail | 대상 | 현재 명령/경계 |
|---|---|---|
| owner-local | 각 티켓의 변경 crate/consumer와 직접 fixture | `just dev-rust-tests <package> [consumer]` 또는 정확한 `cargo test -p ... --test ...`; 결과는 diagnostic |
| dev | cross-package checkpoint | `just dev`; bench harness·doc fixture는 제외되므로 해당 변경은 별도 owner check |
| CI profile | 모든 기본 regression과 static/matrix | clean 동일 HEAD의 `just verify-macos-ci`; `test`, `test-rayon`, doctest, rustdoc, bench smoke, consumer MSRV 등 inventory/required 기준. macOS platform skip은 명시 |
| nightly | 제한 modelcheck/TSan/fuzz, coverage, IAI, mutation | 명시적으로 승인된 고비용 rail만 수행. 일부 실행·중단·coverage 수치는 qualification 아님 |
| release | CI+nightly 및 release checklist의 semver/consumer/human 판단 | [release checklist](../../../../release-checklist.md)에 따른 별도 결정. local PASS로 배포/activation을 주장하지 않음 |

## owner-local 실행 후보

아래의 `신규` 파일은 **제안 경로**이며 현재 존재하는 테스트나 실행 결과가 아니다. 실제 구현에서 이름을 바꾸면 `plan.json`/receipt의 선택 경로를 함께 갱신한다.

| 티켓 | 현재 anchor / 신규 제안 | 우선 선택 명령 |
|---|---|---|
| 002 | contract 기존 `contract_builders.rs`; host 신규 `strict_ingress.rs`, `strict_ingress_host.rs` | `cargo test --locked -p taskmesh --test strict_ingress --test strict_ingress_host` + raw contract control |
| 003 | contract 기존 `contract_roundtrip.rs`, `snapshot_oracle.rs`; 신규 `wire_consumer_negatives.rs`(조건부), host 신규 `wire_consumer_surface.rs`(조건부) | `cargo test --locked -p taskmesh-contract --test contract_roundtrip --test snapshot_oracle` + 실제 추가 target |
| 004 | engine 기존 `hardening_child_scope.rs`; 신규 `cross_governor_ids.rs`; host 기존 `hardening_consumer_surface.rs` | `cargo test --locked -p taskmesh-engine --test cross_governor_ids` + consumer/MSRV |
| 005 | host 기존 `hardening_executor_protocol.rs`, `hardening_executor_authority.rs`; 신규 `hardening_executor_capability_snapshot.rs`(조건부) | 해당 host target + 변경된 `just test-rayon`의 실수집 확인 |
| 006 | host 기존 `hardening_deadline_custody.rs`; 신규 `hardening_deadline_response.rs`(조건부) | 해당 host target; 기한 경합은 barrier/주입 clock 우선 |
| 007 | host 신규 `hardening_root_child_scope.rs`(조건부) | `cargo test --locked -p taskmesh --test hardening_root_child_scope` |
| 008 | engine 신규 `hardening_admission_matrix.rs`; host 신규 `hardening_admission_host_capacity.rs` | 각 신규 target의 owner-local Cargo selection |
| 009 | engine 신규 `hardening_queue_history.rs`; host 신규 `hardening_drain_multiwait.rs` | 각 신규 target; 모델은 별도 `just modelcheck` |
| 010 | engine 신규 `hardening_memory_ledger.rs`; contract 기존 `snapshot_oracle.rs` | 각 engine/contract target |
| 011 | bench 기존 `hellgate.rs`; host 신규 `host_open_loop.rs` | `cargo test --locked -p taskmesh-bench --test hellgate` + host 대상 |

경로 표기는 각 crate의 `tests/` 아래다. 새 fixture는 기본 `just test`에 실제 수집되는지, feature/ignored/conditional selector가 빠지지 않는지 확인한다. 위 명령은 **계획**이며 이번 문서 작업에서는 실행하지 않았다.

현재 `.github/workflows/ci.yml`은 `workflow_dispatch` 전용의 release형 workflow다. `pull_request`만 추가하면 mutation/fuzz까지 자동 실행되므로 bounded PR gate가 필요할 때 별도 CI-profile workflow를 설계한다. GitHub required status check는 최신 commit SHA에 결속되며 merge queue를 쓰면 `merge_group` 이벤트도 요구된다. [GitHub 공식 문서](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks).

## 티켓 완료와 통합 완료

- 티켓: `목적 → 변경 파일(기존/신규/조건부) → 구현 순서 → ID별 DoD → 정확한 owner-local 명령 → 인계·중단 조건`을 유지한다. acceptance ID별 계약/fixture/실행 결과와 남은 예외를 기록한다. 실패 후보는 수정 전 반례와 수정 후 동일 반례를 분리한다.
- 통합: 변경된 source tree의 모든 fixture를 실제 선택 gate에서 재실행. clean HEAD의 CI profile receipt가 필수. 서로 다른 HEAD의 owner-local 결과를 합치지 않는다.
- release: 요청된 경우에만 nightly/mutation 등 비용 rail을 완전하게 수행하고 [required.json](../../../../../tools/gates/required.json)의 전 집합과 raw artifact custody를 판정한다.

이 문서 자체와 `validate_plan.py`의 PASS는 계획 무결성만 확인한다. production 구현·테스트·CI·nightly 자격 증거가 아니다.
