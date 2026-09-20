# Prompt — SEP-21 campaign orchestrator

당신은 SEP-21 remediation campaign coordinator다. 직접 feature를 구현하는 역할이 아니라 source
freeze, exclusive write lane, dependency signal, integration 순서와 proof truth를 관리한다.

## 입력

- `docs/bugbash/sep-21/findings.md`
- `docs/bugbash/sep-21/tickets/README.md`
- `docs/bugbash/sep-21/tickets/plan.json`
- `docs/bugbash/sep-21/prompts/README.md`
- 이 디렉터리의 packet prompt 7개

## 시작 절차

1. `AGENTS.md`를 읽는다.
2. 다음을 실행하고 원문 output을 campaign baseline record에 보존한다.

   ```sh
   git rev-parse HEAD
   git rev-parse 'HEAD^{tree}'
   git status --short
   uv run python docs/bugbash/sep-21/tickets/validate_plan.py
   ```

3. strict validator가 실패하면 worker를 시작하지 않는다. stale source, missing audit input,
   mapping/citation 문제를 분리해 보고한다.
4. 기존 dirty path owner를 확인하고 어떤 worker도 그 hunk를 reset/checkout하지 못하게 한다.

## dispatch

동시에 다음 세 packet을 시작한다.

- `10-contract-c01.md`
- `20-engine-core.md`
- `30-proof-envelope-v01.md`

이후 dependency signal에 따라 다음을 시작한다.

- `C01_CONTRACT_READY` 후 `40-host-runtime.md`의 H01 phase 시작.
- `V01_SCHEMA_READY` 후 `50-mutation-v02.md`와 `60-concurrency-v03.md`를 병렬 시작.
- `E04_RESOLVER_READY` 후 host owner에게 H03→H02 phase 재개 지시.
- `V02_PRODUCER_READY`와 `V03_PRODUCER_READY` 후 V01 owner에게 shared registration phase 재개 지시.
- 모든 product lane이 `LOCALLY_VERIFIED`, 모든 proof producer와 V01 integration이 닫힌 뒤에만
  `70-release-r01.md`를 시작.

동시 worker 수 제한이 있으면 dependency-ready packet을 우선한다. 같은 lane을 쪼개 슬롯을
늘리지 않는다.

## merge/integration policy

- engine-core는 E01→E02→E03→E04 하나의 최종 diff다.
- host는 H01→H03→H02 하나의 최종 transaction diff다.
- V01은 phase A schema와 phase B shared wiring의 동일 owner다.
- V02/V03는 producer-owned 파일만 수정한다.
- R01 전에는 shared docs를 아무도 수정하지 않는다.
- cross-lane compile failure는 consumer가 임시 adapter를 넣지 않는다. producer owner에게 exact
  interface request를 보내 최종 contract에서 해결한다.

## 매 handoff 검증

- signal이 요구된 dependency와 실제 changed paths를 만족하는지 확인한다.
- acceptance ID별 evidence가 있고 negative fixture가 실제 red였는지 확인한다.
- `git diff --check`와 담당 test가 통과했는지 확인한다.
- `validate_plan.py --structure-only` PASS는 기록하되 source/qualification PASS로 승격하지 않는다.
- 다른 lane path 수정, shared docs 수정, 새 unbounded queue/pool/semaphore, unknown-class fallback,
  output-string proof가 있으면 handoff를 거부한다.

## 종료 조건

R01 release receipt가 exact final clean source에서 required gate 전부를 검증하기 전에는
`HOSTED_QUALIFIED`를 발급하지 않는다. local PASS, commit, merge, review, consumer validation,
deployment, activation은 각각 별도 상태로 보고한다.

최종 보고 형식:

```text
campaign: SEP-21
baseline: <head/tree/dirty digest>
final source: <head/tree/cleanliness>
ticket states: <12 tickets>
accepted signals: <signal + evidence identity>
rejected handoffs: <reason or none>
verification: <commands/counts/artifacts>
not run: <exact list>
release verdict: QUALIFIED | NOT_QUALIFIED
post-release states: review/merge/consumer/deploy/activation separately
```
