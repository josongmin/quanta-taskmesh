# Prompt — V03 fuzz/model evidence owner

Assigned ticket: `SEP21-V03`.

당신은 fuzz와 Loom/Shuttle evidence producer의 exclusive owner다. `V01_SCHEMA_READY` 뒤 시작하고,
실행 시간이나 exit 0이 아니라 semantic witness와 replay identity를 증명한다.

## 먼저 읽을 것

- `AGENTS.md`
- `SEP21-V03-fuzz-model-evidence.md`
- findings TM21-018,021
- V01 schema, fuzz scripts/targets/corpus, Loom/Shuttle runners/tests

## write scope

- `tools/fuzz/**`, `fuzz/**`
- `tools/modelcheck/**`
- `crates/taskmesh-engine/tests/loom_governance.rs`, `shuttle_governance.rs`, test-only model fixtures
- producer-owned fuzz/model manifests
- V03 ticket 상태와 evidence

수정 금지:

- production semantic을 proof 통과용으로 변경
- shared receipt/gate inventory/Justfile/workflow
- V01/V02 files
- shared docs

## 구현 요구

1. required target exact set, target별 runs>0, duration>0을 강제한다.
2. wire formats의 Snapshot/Topology/ClassPolicy valid-path checkpoint와 core transition witness를
   target별 구조화한다.
3. minimal/boundary valid corpus와 corpus digest를 tracked manifest로 보존한다.
4. no-op harness, decode-all-fail, missing target을 CLEAN/PASS로 만들지 않는다.
5. Loom은 model ID, builder bounds, requested/completed permutations를 보존한다.
6. Shuttle은 stable seed, scheduler/config/count와 failure schedule artifact를 보존한다.
7. failure artifact 하나로 clean environment에서 동일 failure를 replay한다.
8. producer manifest만 발행하고 shared wiring은 V01 owner에게 넘긴다.

## 검증

```sh
just fuzz-check
FUZZ_SECONDS=60 just fuzz
just loom
just shuttle
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

플랫폼/tool 부재로 실행하지 못하면 NOT_RUN이다. fixture/parser PASS를 실제 fuzz/model exploration
PASS로 표현하지 않는다.

## handoff

V03-A01~A06을 연결한다.

```text
signal: V03_PRODUCER_READY
producer manifests: <paths/digests/schema>
fuzz targets: <required/executed/runs/checkpoints>
model runs: <model IDs/bounds/requested/completed/seeds>
replay artifacts: <paths/digests/results>
negative fixtures: <results>
not run: <exact list>
shared registration request to V01: <exact fields>
```
