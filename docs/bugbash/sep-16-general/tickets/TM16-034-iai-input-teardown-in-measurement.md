# TM16-034 — IAI governance benchmark가 input governor teardown까지 측정한다

- Severity: P2
- Status: OPEN / source ownership and portable-body probe
- Lane: B — benchmark measurement boundaries
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-bench/benches/iai_governance.rs:74-94`: roundtrip/reject 함수가 `(Governor, TaskSpec)`을, snapshot 함수가 Governor를 값으로 받으며 input을 반환하지 않는다.
- 같은 파일 `:8-10`: setup 제외 후 한 governance op를 측정한다고 설명한다.
- Locked iai-callgrind 0.14.2의 primary `src/lib_bench.rs:206-207`: 기본 EntryPoint는 benchmark function이다. macros 0.5.1 `src/lib_bench.rs:433-434`는 원본 signature/body를 보존한다. 별도 entry-point/teardown 설정은 저장소 bench에 없다.
- [Portable probe](evidence/repro-bench/src/lib.rs): `iai_function_bodies_destroy_input_governor_before_return`. [Build script](evidence/repro-bench/build.rs)가 저장소의 원본 3개 함수 본문을 추출하며 measurement attributes만 제외한다.

## Trigger / 관찰

GovernedState/policy/registry를 소유한 input Governor가 각 함수의 반환 전에 drop된다. Governor가 마지막으로 소유하는 Clock의 Drop marker를 사용한 portable 실행에서 roundtrip/reject/snapshot 모두 function-call 구간 안에 destructor가 실행됨을 확인했다.

실제 Linux/Callgrind instruction 수와 teardown 비중은 미측정이다. 이 probe는 Rust ownership 경계의 증거이지 IAI regression qualification이 아니다.

## 원인 / 영향 / 범위

setup cost는 제외하지만 setup 결과의 파괴 비용은 함수 안으로 들어온다. reject metric에는 governor/policy teardown, snapshot metric에는 input class/permit storage teardown이 섞인다. 지속 중인 governor에서 동일 op를 반복하는 wall-clock hot-path metric과 작업 단위가 다르다. teardown 비중이 변하면 hot-path regression이 희석되거나 무관한 변화가 gate를 red로 만들 수 있다. TM16-018의 verdict 무시와 TM16-020의 threshold/cache 불일치와 별도 원인이다.

## 보완 계획

- benchmark function 밖으로 input ownership을 반환하고 teardown hook에서 파괴하거나 명시적 entry-point로 원하는 op만 측정한다.
- snapshot result 파괴와 input governor 파괴의 포함 여부를 각각 명시한다. `mem::forget`으로 fixture를 무조건 leak시키는 해결은 피한다.
- measurement definition 변경 후 이전 baseline과 비교하지 않고 새 compatible baseline을 등록한다.

## Acceptance / 회귀 검증

- input Governor destruction marker가 measured function 반환 이후에 발생한다.
- Linux에서 op-only와 full-fixture roundtrip을 구분하고 teardown-only 변화가 op metric에 섞이지 않음을 확인한다.
- actual expected verdict/final accounting, canonical threshold 및 compatible baseline도 함께 검증한다.
