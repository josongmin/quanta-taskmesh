# Prompt — V02 mutation evidence owner

Assigned ticket: `SEP21-V02`.

당신은 mutation producer의 exclusive owner다. `V01_SCHEMA_READY`를 받은 뒤 V01 envelope을 소비해
curated mutation과 generated cargo-mutants 결과를 격리·분류·보존한다.

## 먼저 읽을 것

- `AGENTS.md`
- `SEP21-V02-mutation-evidence.md`
- findings TM21-012,017
- V01 schema, examples, current mutation runner/config/tests와 historical audit

## write scope

- `tools/verification/**`
- producer-owned mutation manifest와 tests
- V02 ticket 상태와 evidence

수정 금지:

- production crates
- `tools/qualification/**`, shared `tools/gates/**`
- `Justfile`, shared inventory/receipt/workflow
- source checkout를 직접 변이하는 workflow

## 구현 요구

1. campaign은 isolated source copy/worktree와 dedicated `CARGO_TARGET_DIR`을 사용한다.
2. 시작/종료 source digest, isolation root, command/env/profile/tool identity를 보존한다.
3. baseline red, zero tests, partial completion, signal/timeout을 PASS로 만들지 않는다.
4. curated classifier는 exit/signal, expected exact failure와 unrelated co-failure를 함께 판정한다.
5. generated runner는 cargo-mutants raw artifact와 caught/missed/unviable/timeout denominator를
   구조화한다.
6. equivalent는 자동 PASS가 아니며 mutant ID, reachability 근거, reviewer가 필요하다.
7. abrupt exit/concurrent campaign 뒤 원본 source와 normal target이 오염되지 않게 한다.
8. producer manifest만 발행하고 shared wiring은 V01 owner에게 넘긴다.

## 검증

```sh
uv run pytest -q tools/verification/tests
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

generated sweep는 비용이 크더라도 R01 전 current-source에서 실제 실행되어야 한다. 이번 worker가
완료하지 못하면 runner 구현 PASS와 campaign NOT_RUN을 분리한다.

## handoff

V02-A01~A06을 연결한다.

```text
signal: V02_PRODUCER_READY
producer manifest: <path/digest/schema>
runner tests: <commands/counts>
isolation negatives: <results>
curated result: <counts/raw digest or NOT_RUN>
generated result: <caught/missed/unviable/timeout/equivalent or NOT_RUN>
source before/after: <digests>
shared registration request to V01: <exact fields>
```
