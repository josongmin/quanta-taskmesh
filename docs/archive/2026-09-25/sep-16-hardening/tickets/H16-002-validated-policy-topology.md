# H16-002 — Validated policy·canonical inventory·topology 검증

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-001](H16-001-contracts-and-compatibility.md)
- 원본 finding: [TM16-003](../../../../bugbash/sep-16-general/tickets/TM16-003-invalid-topology-panics.md), [TM16-004](../../../../bugbash/sep-16-general/tickets/TM16-004-policyset-default-inventory-bypass.md)
- 배타적 write lease: `contract`, `engine`, `host`, `rayon`; [적용 순서](EXECUTION.md) 준수

## 목적

모든 production constructor가 동일한 완전 검증을 통과하고 policy/topology를 한 번만 resolve한다.

## 변경 범위

- 기존: [crates/taskmesh-engine/src/shared/mod.rs](../../../../../crates/taskmesh-engine/src/shared/mod.rs)
- 기존: [crates/taskmesh-engine/src/features/inventory/mod.rs](../../../../../crates/taskmesh-engine/src/features/inventory/mod.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-contract/src/topology.rs](../../../../../crates/taskmesh-contract/src/topology.rs)
- 기존: [crates/taskmesh/src/builder.rs](../../../../../crates/taskmesh/src/builder.rs)
- 기존: [crates/taskmesh-rayon/src/lib.rs](../../../../../crates/taskmesh-rayon/src/lib.rs)
- 기존: [crates/taskmesh-engine/tests/config_validation.rs](../../../../../crates/taskmesh-engine/tests/config_validation.rs)
- 기존: [crates/taskmesh-engine/tests/substrate_inventory.rs](../../../../../crates/taskmesh-engine/tests/substrate_inventory.rs)
- 제안 경로: `crates/taskmesh-engine/src/shared/validated_policy.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] RawPolicyConfig/legacy PolicySet과 private ValidatedPolicy를 분리한다. Governor::new가 registry 전체를 검증하고 이후 immutable snapshot을 소유한다.
  → 별도 ValidatedPolicy 타입 대신 `PolicySet` private field + `Governor::new` 전체 검증(immutable)
- [x] canonical builtin presence, map key==record.name, name uniqueness, kind/pool binding을 검사한다. builtin 정의는 하나로 모으되 독립 expected inventory test는 유지한다.
- [x] topology min/max, zero 의미, Semaphore::MAX_PERMITS, usize 변환, Fixed/Auto/reserve 및 fallible OS pool creation을 검증한다.
- [x] available_parallelism은 build당 한 번 읽고 ResolvedTopology를 executor/gates/config에 공유한다. 두 번 읽어 서로 다른 capacity를 얻지 않게 한다.
- [x] test-util feature의 new_unchecked는 격리된 test/probe에만 사용한다. production constructor가 feature 활성화 때문에 검증을 생략하지 않는지 external-consumer compile fixture로 확인한다.
- [x] strict profile의 bounds·supported fairness range 검증 hook을 마련하되 후속 알고리즘이 지원하지 않는 값을 silent normalize하지 않는다.
  → strict profile hook 없음; 미지원 값은 construction에서 거절(weight 0·degrade chain·AuthorityOnly pool limit), silent normalize 없음

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `TopologyConfig::validate` / `try_resolved_cpu_workers`와 `TopologyError`를 추가했다.
  `resolved_cpu_workers`는 이제 total이라 inverted window에서도 panic하지 않는다.
- `Governor::new`가 substrate registry 전체를 검증한다: builtin 존재, map key == record.name,
  kind/pool binding, capability limit이 실제 등록된 pool을 가리키는지.
- `PolicySet`의 registry/capability 필드를 private으로 바꾸고 `Default`를 canonical
  `new`에 위임했다. 빈 inventory governor는 더 이상 구성할 수 없다.
- `available_parallelism`은 `Builder::build`에서 **한 번** 읽어 capability limit과 CPU
  executor가 같은 수를 쓴다.
- negative registry는 `test-util`의 `PolicySet::forge_substrates`로만 만들 수 있다 —
  production 경로에는 구멍을 남기지 않고 validator만 시험한다.

Regression: `crates/taskmesh-contract/tests/topology_validation.rs` (5),
`crates/taskmesh-engine/tests/hardening_policy_inventory.rs` (10).

## 검증 / 완료 조건

- [x] `H16-002-A01` Default/struct literal/clear/forged key/kind/pool 모두 우회 불가
- [x] `H16-002-A02` min>max, Fixed(0), 과대 slot count가 panic 대신 typed Err
- [x] `H16-002-A03` 동일 resolved worker count를 capability gate와 CPU executor가 공유 — 직접 비교
      test(`the_cpu_gate_and_the_rayon_pool_are_sized_from_one_answer`: gate == pool == declaration)와
      좁은 pool 거절(`ExecutorDeclaresFewerWorkers`). `RayonCpuExecutor::from_topology`는 편의 생성자로
      자체 parallelism read를 가지며, builder 경로에서는 선언 대조가 불일치를 잡는다.
- [x] `H16-002-A04` extra inventory는 tracking-only이며 builtin을 가리지 않음
- [x] `H16-002-A05` unchecked constructor는 `test-util` 뒤에만 존재

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test config_validation --test substrate_inventory --test audit_hardening
cargo test -p taskmesh --test config_inventory --test runtime_cpu_executor
cargo test -p taskmesh --features rayon
```

## 호환성 / 실패 모드

- 기존 Default를 지우는 것만으로 public struct literal 우회는 해결되지 않는다.
- Topology slot은 Taskmesh dispatch credit인지 실제 executor thread capacity인지 D05에서 구분한다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

