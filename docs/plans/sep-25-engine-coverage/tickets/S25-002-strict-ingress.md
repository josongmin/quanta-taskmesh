# S25-002 — untrusted JSON 승격 경계

- 상태: PLANNED; 우선: P0; 선행: [S25-001](S25-001-contract-boundaries.md); 소유: contract/ingress owner, host 통합 owner.
- 주 담당 시나리오: D04, D21, D22, D25, H33.
- 현재 근거: `task.rs`의 `parent_awaits` default false/optional `stack_size_bytes`, `validation.rs`의 stage-count 부재, `config.rs`와 topology derived Deserialize, `tests/contract_builders.rs`의 parent identity 누락 반례.

## 목적

public raw DTO의 Serde 호환성을 별도로 유지하면서, 신뢰하지 않는 bytes를 실행/설정으로 승격하는 **단일 명시적 strict ingress**를 정의한다. byte cap은 파싱 전 적용하고, decoder는 중첩 depth·stage count·중복/unknown key를 bounded하게 거절한다. `TaskScope::Child`에서는 `parent_awaits`의 **존재**를 요구한다. strict ingress의 blocking dispatch는 `shared_blocking`과 `requested_stack { stack_size_bytes }`를 구분하는 tag를 요구한다. 이 tag가 없으면 누락된 stack 크기가 의도적 shared 요청인지 오타인지 구별할 수 없으므로 reject한다. 허용된 입력만 validated plan/config로 전환한다. 배포 ingress가 저장소 밖이라면 라이브러리 함수의 경계와 외부 통합 증거를 따로 소유한다.

현재 `TaskSpec`과 `TopologyConfig`는 derived Deserialize를 사용한다. child의 `parent_awaits`는 `#[serde(default)]`, stack은 `Option<u64>` default, topology의 `physical_domains`도 default다. `ValidatedTaskPlan`은 `TaskSpec::validate()`를 적용하지만 누락된 의도를 복원할 수 없다. 따라서 목표는 raw DTO를 전역 변경해 기존 wire를 깨는 것이 아니라 **untrusted bytes가 runtime authority로 들어오는 명시적 경계**를 만드는 것이다.

## 변경 파일

| 구분 | 경로 | 작업 |
|---|---|---|
| 기존 근거 | `crates/taskmesh-contract/src/{task,validation,config,topology}.rs` | Serde/default·stage validation·config 구조와 strict mirror의 필드 집합 대조 |
| 신규 제안 | `crates/taskmesh/src/ingress.rs` | 버전/limits를 받는 `parse_task`·`parse_config`와 typed `IngressError`; JSON 파서는 host/공식 adapter 경계에 둠 |
| 기존 변경 | `crates/taskmesh/src/lib.rs`, `crates/taskmesh/Cargo.toml`, 필요 시 `Cargo.lock` | 공개 entrypoint re-export, `serde_json` production dependency. host가 아닌 별도 공식 adapter 위치로 결정되면 그 crate 경로로 이동하고 계획 갱신 |
| 기존 변경 | `crates/taskmesh/src/builder.rs`, `crates/taskmesh/src/execution_plan.rs` | strict config의 Builder 승격 지점과 선택된 dispatch가 한 번만 plan으로 고정되는지 확인; 필요한 연결만 수정 |
| 신규 fixture | `crates/taskmesh/tests/strict_ingress.rs`, `crates/taskmesh/tests/strict_ingress_host.rs` | wire 오류와 실제 dispatch/cycle의 side effect oracle을 분리 |
| 기존 fixture | `crates/taskmesh-contract/tests/{contract_builders,task_plan_validation,contract_roundtrip}.rs`, `crates/taskmesh/tests/hardening_dispatch_resolution.rs` | raw Serde 호환 control과 current pool 행위 재사용/확장 |
| 공개 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | raw DTO와 strict ingress 적용 범위, error taxonomy, migration 예제 |

`crates/taskmesh-contract`는 현재 production `serde_json` 의존성이 없다. strict 코드를 계약 crate에 넣는 선택은 S25-001에서 별도로 승인받아야 한다. 두 위치에 중복 parser를 만들지 않는다.

## 구현 순서

1. S25-001에서 실제 배포 입력 호출점, wire version, dispatch tag 이름/형태, bytes/depth/stage 예산과 오류 우선순위를 확정한다. 설정은 `RuntimeConfig`만 파싱해 반환하는 것과 `Builder`까지 승격하는 것을 분리한다.
2. `&[u8]` 길이를 **파싱 전** 검사한다. JSON visitor/strict mirror가 객체의 unknown·duplicate key를 모든 중첩 수준에서 거절하고, depth·`stages` 길이를 역직렬화 중 상한으로 자른다. `serde_json::Value`/`BTreeMap`으로 먼저 파싱해 duplicate key를 잃은 뒤 검사하는 구현은 불합격이다. `serde_json` 기본 recursion limit에만 예산을 의존하지 않는다.
3. strict child는 `parent_operation_id`, `parent_stage`, `parent_awaits: bool` 모두 명시적으로 받는다. `false`도 유효한 의도적 선언이다. unknown `parent_await` 등은 오류로 반환한다. `TaskSpec::validate()`와 `ValidatedTaskPlan::try_from`을 다시 통과시킨다.
4. blocking strict DTO에서 `dispatch = shared_blocking | requested_stack { stack_size_bytes }`를 필수로 받는다. tag와 `SubstrateHint`/run entrypoint의 조합을 검증한다. requested-stack은 size가 유효할 때만 `ValidatedDispatchPlan`의 `DedicatedStackThread/Runtime`으로 고정한다. shared 경로는 명시적 선택일 때만 통과시킨다.
5. strict config는 `topology.physical_domains` 오타를 포함한 모든 중첩 unknown/duplicate를 거절한다. 그 뒤 `TopologyConfig::validate`, class/resource/policy와 substrate inventory의 기존 Builder/Governor 검증으로 승격한다. JSON 성공만으로 구성 유효성을 주장하지 않는다.
6. raw DTO는 기존 roundtrip을 유지하고, 공식 entrypoint의 예제/호출자 전환을 문서화한다. 배포 소비자가 외부에 있으면 해당 owner가 strict 호출 receipt를 제공한다.

## DoD

- [ ] `S25-002-D04` 0/1/중복/상한/상한+1 stage와 상한+1 JSON bytes가 parse allocation/validation 경계에서 각각 정해진 오류를 낸다. 상한 수치는 workload 근거로 결정한다.
- [ ] `S25-002-D21` 완전한 config의 unknown key와 `physical_domans` 오타가 strict ingress에서 거절되고, raw Serde의 현재 결과는 별도 control로 고정한다. 배포 승격 시 capacity fallback이 없다.
- [ ] `S25-002-D22` parent identity가 모두 유효한 child JSON에서 `parent_awaits`만 누락·오타 낸 경우 strict 경로가 거절한다. 다른 필드 누락으로 우연히 실패하는 fixture는 불합격이다.
- [ ] `S25-002-D25` blocking stack key 오타/누락을 raw→validated→실제 dispatch/role/physical pool까지 추적한다. strict `requested_stack`의 누락/오타와 dispatch tag 누락은 worker/permit 0, 명시적 `shared_blocking`만 shared 경로다.
- [ ] `S25-002-H33` 단일 slot을 가진 parent/child 실제 await 시나리오에서 raw false fallback과 strict reject를 구별하고, 선언된 await에서만 engine cycle 판정을 기대한다.

`serde_json`은 제안한 host ingress의 production dependency다. 계약 crate나 별도 adapter로 옮기는 결정은 S25-001에서 공개 dependency/feature/MSRV 영향을 검토한 뒤 한다. 모든 Serde consumer를 자동으로 strict 처리했다고 주장하지 않는다. owner-local contract+host fixture → 기본 `test`/CI; 외부 ingress가 따로 있으면 그 consumer receipt가 필요하다.

| Acceptance | 필수 독립 판정 |
|---|---|
| `S25-002-D04` | 0/1/중복/상한/상한+1 stage, 상한 경계 bytes, 깊이 상한±1의 오류가 **어느 단계**에서 나는지 구분. 상한+1 stage가 비례해 거대한 `Vec`/map을 먼저 할당하지 않음. 기존 `MAX_TASK_IDENTIFIER_LEN`과 stage-count cap을 혼동하지 않음. |
| `S25-002-D21` | 완전한 config control과 `physical_domans`·중첩 policy 오타·duplicate key가 raw/strict에서 각각 무엇을 만드는지 확인. strict 오류 후 Builder 생성·worker·capacity 변경 0. |
| `S25-002-D22` | **다른 모든 필드는 유효한** child JSON에서 await 키만 제거/오타. raw `ValidatedTaskPlan`의 false fallback과 strict typed 거절을 별도 assertion. |
| `S25-002-D25` | blocking 오타/누락은 raw decode→validated plan→`run_blocking` 실제 `blocking` role과 `physical.shared_blocking`까지 control로 관측. strict requested-stack은 missing/typo/tag 없음 모두 reject, worker/permit/ticket 0. 명시적 shared만 pooled worker·blocking slot. |
| `S25-002-H33` | 단일 class/role slot, 실제 parent가 child result를 기다리는 barrier fixture. strict 미선언은 입장 전 reject, 선언된 await는 engine cycle reject. 명시적 `false`가 실제 wait에 대해 거짓이라면 엔진이 추론해 구제하지 못함을 외부 caller 책임으로 기록. timeout만으로 cycle 성공을 주장하지 않음. |

## 계획된 검증

owner-local 후보: `cargo test --locked -p taskmesh --test strict_ingress`, `cargo test --locked -p taskmesh --test strict_ingress_host`, raw control은 `cargo test --locked -p taskmesh-contract --test contract_roundtrip --test task_plan_validation --test contract_builders`. `cargo test`는 추후 구현 시에만 실행한다. 기본 `just test`가 신규 host integration target을 수집하는지 S25-012가 확인한다. dependency/API가 바뀌면 `just consumer-msrv`, `just doctest`, `just rustdoc`을 통합 owner에게 넘긴다.

## 인계·중단 조건

contract owner가 strict DTO schema·typed error를 고정한 뒤 host owner가 `ValidatedDispatchPlan`/Builder와 연결한다. `execution_plan.rs`의 공용 부분은 S25-005와 직렬 통합한다. 외부 배포 ingress가 strict API를 호출한다는 receipt가 없으면 “library API 구현”과 “배포 입력 안전”을 분리해 후자는 OPEN으로 남긴다. strict화가 raw `TaskSpec` Deserialize 결과 또는 legacy child wire를 바꾸는 설계라면 S25-001로 되돌아가 버전/semver를 먼저 정한다. stage cap을 direct Rust API에도 강제하려면 별도 호환성 판정을 기록한다.
