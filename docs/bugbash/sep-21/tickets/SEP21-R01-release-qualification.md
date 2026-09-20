# SEP21-R01 — Release compatibility and exact-source qualification

- 상태: PLANNED
- 우선순위: P1 final release blocker
- 포함 finding: TM21-013
- 선행: C01, E01, E02, E03, E04, H01, H02, H03, V01, V02, V03
- write lane: `release`

## 목적

모든 product/proof 변경이 끝난 frozen source에서 semver compatibility와 release-specific proof를
한 번만 판정한다. 일반 CI green, historical receipt, manual checkbox를 release PASS로 합치지 않는다.

## RCA

- README가 매 release semver 검사를 약속하지만 qualification inventory에 gate가 없다.
- public API/wire/behavior blind spot의 human adjudication artifact가 없다.
- continuous qualification과 release decision의 required set이 구분되지 않는다.
- historical local receipt와 current dirty source가 함께 존재해 closure 경계가 불명확하다.

## 확정 근거

- release마다 semver check를 약속하는 문서: `README.md:112-122`.
- manual checklist만 존재: `docs/release-checklist.md:124-144`.
- `Justfile`, gate inventory와 qualification workflow에는 semver release gate가 없다.
- root historical receipt source는 `cc5b256704a1826e7dde36e2c4b127c6cba8aabf`이고
  verdict는 `NOT_QUALIFIED`; current HEAD와 다르다.

## 목표 구조와 불변식

- release profile은 immutable baseline SHA와 current exact SHA를 입력으로 받는다.
- 공개 4 crate 각각의 semver raw result와 facade/wire/behavior reconciliation을 보존한다.
- 모든 upstream ticket artifact digest와 source identity가 current release source와 같아야 한다.
- required missing/NOT_RUN/SKIPPED/TIMEOUT/stale/wrong-SHA는 `NOT_QUALIFIED`다.
- coverage는 descriptive evidence이며 percentage를 correctness gate로 승격하지 않는다.
- hosted qualification, review/merge, consumer qualification, activation은 별도 상태다.

## 작업 플랜

1. 신규 `tools/release/semver.py`, `release-policy.json`, tests
   - baseline ref를 immutable SHA로 resolve하고 4 crate를 exact tool/features로 검사한다.
2. `Justfile`, `tools/gates/inventory.json`, 신규 `release-required.json`
   - ordinary required set과 release required set을 분리한다.
3. 신규 `.github/workflows/release.yml` 또는 isolated release job
   - V01 pinned/action envelope 위에서 clean checkout만 허용한다.
4. `tools/qualification/receipt.py` release profile 또는 별도 `tools/release/receipt.py`
   - semver raw outputs, manual reconciliation, upstream artifact digest graph를 결합한다.
5. `README.md`, `CHANGELOG.md`, `docs/release-checklist.md`,
   `docs/taskmesh-external-interface.md`, `docs/taskmesh-library-spec.md`, 관련 ADR
   - C01/E03/H01/H02/H03 closure evidence의 public migration, semantic contract, rollback,
     consumer adoption을 최종 source 기준으로 한 번만 통합한다.
6. generated cargo-mutants는 이번 engine/host 대수술 release에서 V02의 current-source
   `REPORTED`를 필수로 한다. curated 결과와 별도 표시한다.
7. coverage manifest에는 source/tool/feature/test scope와 line/region/function/instantiation/
   branch/MCDC collected 여부 및 raw digest를 연결한다.

## release artifact

- `release-receipt.json`
- crate별 semver raw output + `semver-manifest.json`
- facade re-export, serde/wire, return type, behavior blind-spot reconciliation
- V01/V02/V03 및 product regression receipt digest set
- coverage/fuzz/model/mutation/performance raw artifact digests

## negative fixture

- semver tool 없음, public crate 하나 누락, movable baseline ref.
- raw output 또는 human blind-spot manifest 누락.
- accepted break에 CHANGELOG/reviewer 없음.
- upstream artifact source SHA mismatch.
- continuous qualification만 있고 release receipt 없음.
- generated mutation/coverage 상태를 curated PASS로 승격.
- historical `cc5b256...` receipt를 current source에 재사용.

## DoD

- `SEP21-R01-A01`: baseline/current SHA, release type, tool version, features가 machine-readable하다.
- `SEP21-R01-A02`: C01/H03 break가 expected/approved 또는 fixed incompatibility로 판정된다.
- `SEP21-R01-A03`: full workspace, default/rayon/MSRV, Loom/Shuttle/TSan, curated+generated mutation,
  non-vacuous fuzz, IAI comparison을 frozen clean source에서 완료한다.
- `SEP21-R01-A04`: receipt validator가 모든 negative fixture를 red로 만든다.
- `SEP21-R01-A05`: QUALIFIED가 review/merge/consumer/activation을 자동 완료 처리하지 않는다.
- `SEP21-R01-A06`: 23건이 regression/negative proof와 exact ticket closure에 연결된다.

## 금지되는 임시방편

- semver checkbox만 추가.
- `cargo semver-checks` exit code만 보존하고 baseline/tool/raw output 누락.
- local dirty receipt를 hosted-qualified로 표시.
- manual waiver를 source/reviewer/expiry 없이 PASS로 처리.

## 검증 명령 후보

```sh
just gate
just matrix
just proof
just semver-release
uv run python tools/qualification/receipt.py validate release-receipt.json
```
