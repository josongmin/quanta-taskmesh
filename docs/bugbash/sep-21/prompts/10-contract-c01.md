# Prompt — C01 task-plan contract owner

당신은 `SEP21-C01`의 exclusive `contract-task` owner다. product-neutral provenance, validated task
plan, immediate-parent lineage contract를 최종 형태로 구현한다. engine/host에 임시 호환층을 만들지
않는다.

## 먼저 읽을 것

- `AGENTS.md`
- `docs/bugbash/sep-21/tickets/SEP21-C01-task-plan-contract.md`
- `docs/bugbash/sep-21/findings.md`의 TM21-010과 E04가 소비할 TM21-001/TM21-023 contract 요구
- `crates/taskmesh-contract/src/task.rs`
- 현재 serde/contract tests와 public exports

## write scope

- `crates/taskmesh-contract/src/task.rs`
- 필요 시 `crates/taskmesh-contract/src/validation.rs`
- `crates/taskmesh-contract/src/lib.rs`
- contract-owned tests/fixtures
- C01 ticket의 상태와 closure evidence

수정 금지:

- `crates/taskmesh-engine/**`
- `crates/taskmesh/**`, `crates/taskmesh-rayon/**`
- shared README/CHANGELOG/spec/ADR
- proof/receipt/CI files

## 구현 요구

1. raw `TaskSpec`과 admission 가능한 validated plan을 타입으로 분리한다.
2. provenance는 bounded, validated, product-neutral wire contract로 만든다. Search/Index/SDK
   taxonomy를 core branching에 남기지 않는다.
3. zero stage, invalid identifier, duplicate stage, conflicting descriptor, incomplete reduce,
   invalid parent identity를 한 contract validator가 판정한다.
4. child scope가 root뿐 아니라 immediate parent operation과 parent stage를 보존하게 한다.
5. builder, manual construction, serde decode가 validator를 우회할 수 없거나 raw/validated 경계를
   명확히 강제하게 한다.
6. old wire migration과 Rust source break를 숨기지 말고 explicit deprecated window 또는 explicit
   breaking rejection으로 고정한다.
7. `verdict.rs`와 executor contract는 건드리지 않는다. 필요한 error는 validation module이
   소유한다.

## 검증

```sh
CARGO_TARGET_DIR=target/sep21/contract-c01 cargo test --locked -p taskmesh-contract
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

positive roundtrip만으로 끝내지 않는다. duplicate same/conflicting descriptor, empty/oversize key,
ambiguous parent, invalid serde input이 typed error이고 stateful crate 없이 판정되는 negative fixture를
실행한다.

## handoff

C01-A01~A07을 각각 evidence에 연결한다. 완료되면 coordinator와 engine/host owner에게 다음을
보낸다.

```text
signal: C01_CONTRACT_READY
public types: <exact symbols>
wire compatibility: <preserved/deprecated/breaking details>
changed paths: <list>
tests: <command, exit, selected/executed count>
negative fixtures: <list>
documentation delta for R01: <before/after>
unresolved: <none or exact blocker>
```

focused contract PASS는 engine integration이나 release qualification이 아니다.
