# Prompt — V01 trusted evidence envelope owner

Assigned ticket: `SEP21-V01`.

당신은 shared proof schema와 qualification trust root의 단일 owner다. V01을 phase A와 phase B로
나눠 수행하되 두 phase 모두 같은 owner가 통합한다.

## 먼저 읽을 것

- `AGENTS.md`
- `SEP21-V01-trusted-evidence-envelope.md`
- findings TM21-011,014,022
- current receipt, gate inventory/runner, IAI scripts/config, CI/bench workflows와 tests
- V02/V03 ticket의 producer manifest 요구

## write scope

- `tools/qualification/**`
- shared `tools/gates/**`
- IAI runner/config/tests
- `.github/workflows/ci.yml`, `bench.yml`의 trust/proof wiring
- V02/V03 producer가 완료된 뒤 `Justfile`, shared inventory/receipt/workflow registration
- V01 ticket 상태와 evidence

수정 금지:

- production crates
- V02의 `tools/verification/**`
- V03의 `tools/fuzz/**`, `tools/modelcheck/**`, `fuzz/**`
- shared product docs

## phase A — schema ready

1. canonical evidence envelope에 source/tool/config/action/artifact/result identity와 selected/executed
   counts를 정의한다.
2. missing identity, missing raw artifact, digest mismatch, required NOT_RUN/SKIPPED/TIMEOUT을
   fail-closed 판정한다.
3. IAI baseline generation과 verified old-vs-new comparison을 별도 상태로 만든다. stamp-only cache는
   qualification이 아니다.
4. mutable action ref와 PR write credential을 trust root에서 제거한다.
5. V02/V03가 구현할 producer manifest schema와 fixture examples를 고정한다.

완료 시 다음을 보낸 뒤 V02/V03 producer를 기다린다.

```text
signal: V01_SCHEMA_READY
schema/version: <exact file and digest>
required producer fields: <list>
example valid/invalid manifests: <paths>
shared files reserved by V01: <list>
tests: <commands/counts>
```

## phase B — shared integration

`V02_PRODUCER_READY`와 `V03_PRODUCER_READY`를 모두 받은 뒤:

1. 두 producer manifest를 shared inventory, receipt, Justfile, CI에 한 번만 등록한다.
2. producer status/count/artifact와 combined receipt의 1:1 parity를 검증한다.
3. raw/envelope artifact를 failure에도 보존한다.
4. stale sidecar, forged summary, missing producer, action/tool/config mismatch negative fixture를 실행한다.

## 검증

```sh
uv run pytest -q tools/qualification/tests tools/gates/tests tools/bench/tests/test_iai_gate.py
uv run python tools/gates/validate_inventory.py
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

실제 Linux IAI 비교를 실행하지 못했다면 NOT_RUN으로 남긴다. fixture가 schema validation을
통과했다는 사실을 performance PASS로 표현하지 않는다.

## closure

V01-A01~A07을 연결하고 다음을 보낸다.

```text
signal: PROOF_ENVELOPE_CLOSED
schema/tool/action identities: <digests>
producer registrations: <V02/V03 parity>
negative fixtures: <results>
workflow permission/pin audit: <result>
not run: <exact list>
```
