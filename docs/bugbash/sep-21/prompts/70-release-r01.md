# Prompt — R01 release qualification owner

Assigned ticket: `SEP21-R01`.

당신은 SEP-21 release closure의 마지막 단일 owner다. 모든 product/proof lane closure를 입력으로
받아 exact final source에서 semver, full matrix, proof artifact, shared documentation을 통합한다.
선행 source를 직접 고쳐서 green으로 만들지 않는다.

## 시작 gate

다음이 모두 있어야 시작한다.

- C01, E01–E04, H01–H03가 `LOCALLY_VERIFIED`
- V02/V03 producer-ready와 V01 phase B shared integration 완료
- 각 ticket acceptance evidence, changed paths, negative fixtures, doc delta
- final candidate HEAD/tree와 clean source policy

하나라도 없으면 `NOT_QUALIFIED`로 종료하고 원 owner ticket을 reopen한다.

## 먼저 읽을 것

- `AGENTS.md`
- `SEP21-R01-release-qualification.md`
- 모든 ticket closure evidence
- V01/V02/V03 manifests와 raw artifacts
- release checklist, public API docs, semver policy, current gate inventory/workflows

## write scope

- release semver runner/policy/tests
- release-only workflow/required set/receipt
- `README.md`, `CHANGELOG.md`, release checklist, external interface, library spec, 관련 ADR
- R01 ticket status/evidence

수정 금지:

- product/engine/host implementation
- producer result를 재분류하거나 raw evidence 없이 PASS 합성
- predecessor acceptance를 R01에서 완화

## 실행

1. baseline ref를 immutable SHA로 resolve하고 public 4 crate의 tool/features/raw output을 보존한다.
2. facade re-export, serde/wire, return type, behavioral break blind spot을 human adjudication manifest에
   연결한다.
3. ordinary required set과 release required set을 분리한다.
4. final clean checkout에서 full workspace, default/rayon/MSRV, Loom/Shuttle/TSan, curated+generated
   mutation, non-vacuous fuzz, IAI comparison을 실행한다.
5. coverage는 source/tool/feature/test scope와 raw digest, line/region/function/instantiation/branch/MCDC
   collected 여부를 기록한다. 미수집 지표를 0이나 PASS로 표현하지 않는다.
6. 23 findings를 regression/negative evidence와 exact ticket closure에 연결한다.
7. product ticket이 준 doc delta를 final API와 다시 대조해 shared 문서를 한 번만 갱신한다.
8. required NOT_RUN/SKIPPED/TIMEOUT, source/tool/config mismatch, missing raw artifact가 하나라도 있으면
   release receipt는 NOT_QUALIFIED다.

## 검증 후보

```sh
just gate
just matrix
just proof
just semver-release
uv run python tools/qualification/receipt.py validate release-receipt.json
uv run python tools/gates/validate_inventory.py
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

실제 repository에 등록된 canonical commands를 사용한다. 존재하지 않는 command를 임의 PASS로
대체하지 않는다.

## 최종 판정

R01-A01~A06이 모두 충족되고 hosted exact-source receipt가 있을 때만 R01을
`HOSTED_QUALIFIED`로 바꾼다.

```text
release verdict: QUALIFIED | NOT_QUALIFIED
final source: <HEAD/tree/cleanliness>
baseline: <immutable SHA>
ticket closure: <23 findings/12 tickets>
required gates: <status/count/artifact digests>
semver: <crate results + adjudication>
mutation/fuzz/model/perf/coverage: <raw-linked results>
not run/skipped/timeout: <exact list>
review: <state>
merge: <state>
consumer qualification: <state>
deployment: <state>
activation: <state>
```

`QUALIFIED`는 review/merge/consumer/deployment/activation 완료를 의미하지 않는다.
