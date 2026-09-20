# SEP21-V01 — Trusted execution and evidence envelope

- 상태: IN_PROGRESS
- 현재 단계: phase A schema ready; phase B producer registration 대기
- 우선순위: P1 release blocker
- 포함 finding: TM21-011, TM21-014, TM21-022
- 선행: 없음
- write lane: `proof-envelope`
- 후속 소비자: V02, V03, R01

## 목적

모든 gate가 공통 source/tool/config/artifact envelope를 발행하게 하고, IAI stamp·mutable action
tag·불완전 tool metadata가 PASS authority를 갖지 못하게 한다.

## RCA

- IAI는 baseline artifact가 아니라 fingerprint stamp 존재로 comparison을 추론한다.
- receipt는 gate별 actual tool/config/advisory DB identity를 모른다.
- qualification workflow가 mutable action/tag/tool channel에서 실행된다.
- PR benchmark compare와 trusted-main publish가 같은 write-capable job에 섞여 있다.
- 중앙 collector가 producer의 semantic completion을 검증할 공통 manifest가 없다.

## 확정 근거

- fingerprint stamp만으로 pre-run QUALIFIED 결정:
  `tools/bench-iai.sh:111-121`.
- benchmark exit 0 뒤 artifact manifest 없이 stamp만 기록:
  `tools/bench-iai.sh:124-144`.
- fake benchmark artifact 없이 두 번째 run QUALIFIED를 요구하는 test:
  `tools/bench/tests/test_iai_gate.py:198-212,280-291`.
- receipt environment identity 범위: `tools/qualification/receipt.py:176-186`.
- mutable CI actions/toolchain channel: `.github/workflows/ci.yml:27-38,159-201`.
- PR에도 write permission을 가진 benchmark job:
  `.github/workflows/bench.yml:9-12,122-170`.

## 목표 구조와 불변식

- 각 producer는 공통 `EvidenceEnvelopeV1`에 source, command, tool, config, raw artifact,
  selected/executed count, start/end/exit/status를 기록한다.
- 중앙 receipt는 raw log 의미를 재해석하지 않고 producer manifest schema/digest/status만 검증한다.
- IAI는 verified baseline manifest와 actual comparison completion이 모두 있어야 QUALIFIED다.
- 모든 action은 reviewed full commit SHA, workflow 기본 권한은 `contents: read`다.
- write/publish는 trusted main job/environment만 수행한다.

## 작업 플랜

1. 신규 `tools/qualification/evidence.py`
   - canonical JSON, digest, artifact list/size, tool/config identity schema를 구현한다.
2. `tools/qualification/receipt.py`, tests
   - gate별 envelope completeness, source equality, digest linkage를 검증한다.
3. `tools/bench-iai.sh`, `tools/bench/iai_gate.py`, `perf-gate.json`
   - baseline source/tool/config/artifact manifest를 생성·검증한다.
   - runner output에서 actual old-vs-new comparison completion을 구조화한다.
4. `.github/workflows/ci.yml`, `bench.yml`
   - action full SHA pin, top-level read permission, PR compare/main publish 분리.
   - raw/envelope artifacts를 failure에도 `always()` 업로드한다.
5. `tools/gates/validate_inventory.py`, `tools/gates/run.py`, inventory tests
   - mutable action ref, missing envelope/tool identity, excessive permission을 fail한다.
6. tool manifest에 stable/nightly/MSRV `rustc -Vv`, cargo tools, uv/Python, valgrind,
   semgrep, advisory DB revision과 config digest를 기록한다.
7. V02/V03 producer manifest가 고정된 뒤 V01 owner가 Justfile, inventory, receipt, CI registration을
   한 번에 통합한다. V02/V03가 shared orchestration을 직접 편집하지 않게 한다.

## 산출 artifact

- `tools-manifest.json`, `workflow-manifest.json`
- `target/iai/baseline-manifest.json`
- IAI raw comparison output + digest
- gate별 `evidence-envelope.json`
- combined/gates/mutations/coverage/fuzz/model receipts의 digest graph

## negative fixture

- matching stamp만 있고 baseline artifact 없음/손상/부분 cache.
- runner exit 0이나 comparison completion 없음.
- baseline source/tool/config digest mismatch.
- `uses: action@vN`, `@stable`, mutable tool install.
- PR job write token 또는 third-party action으로 write token 전달.
- required tool/config/workflow/artifact identity 누락·변조.
- producer manifest와 combined receipt status/count 불일치.

## DoD

- `SEP21-V01-A01`: baseline 생성 run은 QUALIFIED가 아니며 verified comparison만 QUALIFIED다.
- `SEP21-V01-A02`: 같은 source라도 action/tool/config가 바뀌면 별도 evidence identity다.
- `SEP21-V01-A03`: required identity를 수집할 수 없으면 PASS가 아니라 NOT_RUN/FAIL이다.
- `SEP21-V01-A04`: PR code와 third-party compare action이 write credential을 받지 않는다.
- `SEP21-V01-A05`: stale sidecar, stamp-only cache, forged summary negative가 모두 red다.
- `SEP21-V01-A06`: V02/V03가 별도 schema를 만들지 않고 공통 envelope를 사용한다.
- `SEP21-V01-A07`: V02/V03 manifest의 required artifact/status/count가 inventory와 1:1이며 missing producer는
  qualification PASS가 아니다.

## 금지되는 임시방편

- stamp 파일에 필드를 더 넣는 것만으로 artifact manifest를 대체.
- log substring을 combined receipt가 직접 파싱.
- tag pin을 version comment로만 보완.
- PR job 안에서 `if: main`으로 write token을 계속 보유.

## 검증 명령 후보

```sh
uv run pytest -q tools/qualification/tests tools/gates/tests tools/bench/tests/test_iai_gate.py
uv run python tools/gates/validate_inventory.py
just bench-iai
```

## Phase A evidence (2026-09-21)

- schema: `tools/qualification/evidence-envelope-v1.schema.json` v1
  (`sha256:52149f75ae4b954a7844582cd5bb53a20eb464253666681e0f1876a73ea806bb`).
- validator: `tools/qualification/evidence.py`; canonical JSON/digest, source/command/tool/config/
  workflow/action/artifact/result identity, selected/executed parity, UTC ordered interval을 검증한다.
- producer fixtures:
  `tools/qualification/examples/v02-mutation-valid.json`,
  `tools/qualification/examples/v03-fuzz-valid.json`,
  `tools/qualification/examples/invalid-missing-raw-artifact.json`.
- IAI: `baseline-manifest.json`의 raw `.out` size/digest와 exact tool/config identity가 먼저
  검증되고, 현재 run의 `summary.json`마다 old/new `Both` metric이 있을 때만 `QUALIFIED`다.
  baseline 생성은 `BASELINE_CREATED`, stamp-only/missing/corrupt/incomplete evidence는 non-pass다.
- workflow trust: 모든 action full-SHA pin, top-level `contents: read`, PR trend compare token 제거,
  write/publish는 trusted-main + `benchmark-publish` environment로 분리했다.
- phase B 보류: V02/V03 producer manifest를 shared receipt/inventory/Justfile/CI에 아직 등록하지
  않았다. `V02_PRODUCER_READY`와 `V03_PRODUCER_READY` 둘 다 수신한 뒤 V01 owner가 통합한다.
