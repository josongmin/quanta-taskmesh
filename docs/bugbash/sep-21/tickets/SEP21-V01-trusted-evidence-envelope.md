# SEP21-V01 — Trusted execution and evidence envelope

- 상태: IMPLEMENTED_UNQUALIFIED
- 현재 단계: phase B source integration with focused local proof; hosted qualification and V02 full generated sweep pending
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
  (`sha256:a536a78b0eb264674a9356ed3e437a709df5bfba8a8fe75640b3b2488b45a018`).
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

## Phase B evidence (2026-09-21)

- exact integration base: `main@39690639f94f1c264dc40fbba6b6d9fcaea337c0`.
- canonical registrations: `tools/gates/inventory.json` + `tools/gates/required.json` register exactly
  four surfaces: `mutation-campaign-curated-v1`, `mutation-campaign-generated-v1`,
  `taskmesh-fuzz-v03`, `taskmesh-modelcheck-v03`. Curated and generated mutation denominators
  remain separate required gates (`mutants-critical`, `mutants-generated`).
- producer manifest identities:
  - V02 `tools/verification/mutation-gate.json`:
    `c861ef1a959715f1de71f5442204b4b45aae49bd40ddb532b21f0164cec2b8bd`.
  - V03 fuzz `tools/fuzz/producer-manifest.json`:
    `352ef5bcb6720fe2fa53538c5382a21a0043d71def31fb71e2bf46ecd51fe363`.
  - V03 model `tools/modelcheck/producer-manifest.json`:
    `4a79af53acfd704663bd038c57bb5d3aecdf3801a106e279005d9a31870692c2`.
- combined receipt schema v3 embeds the four producer records and writes exact
  `receipt.gates.json` / `receipt.producers.json` sidecars. Validation re-hashes manifest,
  summary, envelope, raw/replay artifact identities and requires disk sidecar payload equality,
  exact source/config/tool/action identity, exact gate result/exit/envelope parity, and producer
  semantic completion. It does not parse raw logs.
- hosted action authority is runtime-bound: every producer and the collector derive
  `workflow_ref`, workflow digest, `GITHUB_JOB`, `GITHUB_EVENT_NAME`, and exact `GITHUB_SHA`
  from the executing Actions process. A hosted receipt accepts only producer envelopes from
  the same workflow/ref/event/source and each registration's actual producer job. Explicit
  `local` action contexts remain non-qualifying.
- V02 campaign and qualification receipt use the same canonical tracked+nonignored source digest
  primitive. An end-to-end fixture builds real V02 envelopes, reads them through `collect_records`,
  accepts the matching tree, and rejects a one-byte tracked-source drift.
- producer-output cleanup resolves every registered output under repository `target/` before
  unlinking; absolute paths, `..`, the target root, directories, source paths, and symlink
  escapes reject without deletion.
- the generic gate runner uses process groups and bounded TERM-to-KILL cleanup. Curated and
  generated mutation denominators run in independent jobs/artifacts; the generated workspace
  gate has an explicit 21,000-second bound under its own 360-minute job. Qualification always
  runs after producer failures, tolerates missing downloads long enough to emit NOT_QUALIFIED,
  imports the exact registered artifacts, and bounds its remaining sequential critical path to
  18,000 seconds under a 360-minute job. Timeout/signal state is recorded as FAIL evidence.
- workflow integration: `mutants-critical`, `mutants-generated`, `fuzz`, and `modelcheck` execute
  canonical Justfile recipes; their raw/summary/envelope directories upload under `if: always()`.
  The hosted qualification artifact includes both mutation sidecars, producer graph, V02/V03 raw
  evidence, and IAI manifests/log.
- negative coverage: missing/duplicate/stale producer; wrong source/config/tool/action/artifact;
  embedded digest/result drift; PASS with null/nonzero exit; generated/curated conflation; generated
  subset; V03 zero/partial semantic witnesses; incomplete model set and replay mismatch all reject.
- local verification:
  - `uv run pytest -q tools/qualification/tests tools/gates/tests tools/bench/tests/test_iai_gate.py`
    -> `167 passed`.
  - `uv run pytest -q tools/fuzz/tests tools/modelcheck/tests tools/verification/tests`
    -> `73 passed`.
  - focused curated mutations
    `receipt-validator-trusts-a-detached-mutation-digest` and
    `receipt-claims-generated-mutation-pass-without-evidence` -> `2/2 KILLED`.
  - `uv run ruff check tools` -> PASS.
  - `uv run python tools/gates/validate_inventory.py` -> `26 gates, 26 required, 26 workflow
    invocations`, PASS.
  - `uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only` -> PASS
    (`tickets=12 packets=7 findings=23 acceptance=71 links=24 acyclic=true`).
  - `git diff --check` -> PASS.
- DoD mapping:
  - A01-A04 remain covered by phase A IAI/trust-root evidence.
  - A05: stale combined sidecars and forged producer summaries are red in receipt negatives.
  - A06: all V02/V03 surfaces consume `EvidenceEnvelopeV1` v1; no parallel schema is trusted.
  - A07: inventory/required/receipt/workflow enforce the same four registrations and exact
    status/count/artifact identity.
- NOT_RUN / non-closure:
  - hosted `collect --hosted-ci` on an isolated clean checkout is NOT_RUN locally.
  - V02 full-workspace generated cargo-mutants campaign is NOT_RUN; the recorded package subset is
    FAIL and cannot satisfy `mutants-generated` or be combined with the curated denominator.
  - actual Linux IAI old-vs-new comparison is NOT_RUN locally.
  - therefore no `QUALIFIED` receipt or release closure is claimed.

## Phase B adversarial audit correction (2026-09-21)

- The V02 campaign and V01 receipt now hash the identical Git source-path set, including three
  tracked historic receipts. `collect` re-hashes producer sidecars and raw/replay artifacts on disk
  before verdict; `validate` also requires mutation sidecar parity with the embedded receipt.
- Curated PASS binds the complete ordered mutation IDs and inventory digest from the actual
  `tools/verification/mutations.json`, not a producer-asserted denominator. Generated PASS binds
  summary argv to the envelope command, planned IDs to categorized outcomes and structured
  cargo-mutants raw JSON/category files, and exactly one successful baseline digest. A partial
  generated sweep cannot borrow the curated denominator or report PASS without identities.
- Hosted qualification consumes `needs` results from its actual workflow runtime and requires
  `success` for every registered producer job. A producer envelope uploaded before a later job
  failure cannot qualify. Imported gate rows explicitly record `imported_producer=true`, the
  registered envelope path/digest, and envelope start/end timestamps; the validator enforces this
  provenance rather than presenting imported work as direct collector execution.
- The qualification collector step itself has `if: always()` and uses baseline `python3`. Missing
  optional setup tools become recorded gate FAIL or producer NOT_QUALIFIED. Checkout/runner failure
  remains irreducible because the collector code is then unavailable on disk. The workflow
  validator checks the step-level `always()` and runtime `needs` binding.
- Valid-JSON malformed producer envelopes, summaries, manifests, record identities, gate IDs,
  and sidecars return NOT_QUALIFIED reasons rather than escaping as exceptions. The collector
  writes a durable NOT_QUALIFIED receipt for malformed imported evidence. Negative tests cover
  missing/failed/cancelled prerequisite jobs, absent imported provenance, and 144 nested shape
  mutations; none can become QUALIFIED.
- V03 fuzzy/model semantic identity is one-to-one: duplicate target/model rows cannot overwrite a
  failing row, required/executed target names are unique, and Loom manifest model IDs are unique.
  The model validator derives the complete producer-emitted projection from each manifest checker:
  Loom ID/checker/bounds plus positive completion, Shuttle ID/checker/scheduler/seed/requested/
  completed/max_steps, and replay identity plus the unique envelope artifact matching the manifest
  schedule glob. Loom scheduler is intentionally not inferred from absent summary fields; its
  exhaustive execution remains V03 producer-owned proof. Single-field negative tables cover every
  projected model/replay field. Generated raw artifact summary paths are unique, match the full
  raw disk file set, and bind every file's size/digest; duplicate or unknown raw paths reject.
- Local checks on the integrated dirty tree at `main@39690639f94f1c264dc40fbba6b6d9fcaea337c0`:
  - `uv run pytest -q tools/qualification/tests/test_receipt.py tools/gates/tests/test_inventory.py`
    -> `194 passed`.
  - `uv run pytest -q tools/fuzz/tests tools/modelcheck/tests tools/verification/tests`
    -> `74 passed`.
  - `uv run pytest -q tools -m 'not slow' --maxfail=10` before the final model projection
    -> `441 passed, 1 failed, 2 deselected`: a V02 test killed its child after a fixed 0.1-second
    sleep before the child wrote the mutation. The exact test passed on isolated rerun. The V02
    owner replaced the timing assertion with a child-ready handshake. The final whole-tools
    rerun result is reported in the source-bound V01 handoff, not written back into this hashed
    ticket after execution. This earlier failure is not counted as V01 PASS.
  - `uv run ruff check tools`, `uv run python tools/gates/validate_inventory.py`,
    `uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only`,
    and `git diff --check` -> PASS; inventory parity `26/26/26`.
  - 105/105 curated mutation find anchors matched exactly once after the engine source freeze.
    The focused `nested-wait-counts-siblings-as-the-parent` mutation was KILLED on the
    pre-correction source. The final exact-tree rerun outcome is reported in the V01 handoff
    rather than written back into this source-hashed ticket after execution.
- Lifecycle remains `IMPLEMENTED_UNQUALIFIED`: no clean hosted qualification receipt, no full
  generated workspace sweep, and no actual Linux IAI comparison are available. The observed
  generated package subset includes compile-impossible unviable outcomes and is FAIL under the
  current all-caught policy; policy changes belong to R01 after the full sweep, not this audit.
