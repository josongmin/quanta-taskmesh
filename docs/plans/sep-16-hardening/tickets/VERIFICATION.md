# 검증·증거 계약

## 이번 문서 작업의 증거 경계

- current source/manifest/commands와 기존 audit 문서를 정적으로 대조했다. 두 독립 reviewer는 runtime/engine과 toolchain/gate/proof 관점의 설계 누락을 검토했다.
- 이 작업에서 production Rust 테스트·benchmark·strict CI 전체를 재실행하지 않는다. 이전 repro PASS를 새 head의 수정 완료로 사용하지 않는다.
- 이 디렉터리의 validator는 계획 ID, source hashes, dependency closure, local links, acceptance IDs와 파일 존재를 검증한다. algorithm correctness나 production safety를 검증하지 않는다.
- 새 production 모듈·test·gate runner 경로는 각 티켓에 '제안 경로'로 명시했다. 지금 callable하다고 주장하지 않는다.

## 현재 존재하는 실행 진입점

아래는 현재 source에서 확인한 command inventory이며 이번 turn의 실행 결과 목록이 아니다. gate가 현재 red일 수 있다. feature/env/platform/actual collection을 구현 receipt에 고정한다.

```sh
just gate
just proof
just clippy
just deny
just semgrep
just test-architecture
just py-lint
just py-test
just bench-gate
just bench-iai
just loom
just shuttle
cargo check --workspace
cargo test -p taskmesh --features rayon
cargo test -p taskmesh-rayon
cargo test --doc -p taskmesh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo bench -p taskmesh-bench -- --test
```

`just mutants-critical`(tools/verification/run_mutations.py, `runner: cargo|pytest`), inventory runner/parity checker(tools/gates), receipt validator(tools/qualification/receipt.py), external consumer fixture(tools/consumer-msrv)는 이 plan 안에서 구현되었다. `just cov-gate`는 없으며 coverage gate는 이 저장소의 약속이 아니다. 외부 activation verifier는 구현된 도구가 아니다.

## acceptance proof

1. source finding의 observable failure를 production entry point에 재현한다. hang/crash test는 isolated subprocess와 상한을 둔다.
2. 수정된 원본 test의 정상 control이 PASS하고 test collection/핵심 operation count가 양수인지 확인한다.
3. 원래 faulty behavior와 같은 **의도된 mutation 하나만** 적용한 대상에서 expected assertion이 FAIL해야 한다. compile error, malformed fixture, unrelated panic, timeout, zero tests는 mutation kill로 인정하지 않는다. 본래 hang을 검증하는 bounded subprocess의 expected timeout 판정은 별도 명명된 hang test 계약으로만 허용한다.
4. mutation을 제거한 동일 source에서 control을 다시 검증한다. 최종 receipt에는 원본/source/mutation digest와 기대 실패 이유가 있다.
5. Loom/Shuttle은 production transition seam을 사용하고 독립 oracle과 비교한다. 작은 model이 외부 executor/OS/drop 전체를 증명한 것처럼 보고하지 않는다. [Loom의 공식 synchronization instrumentation 설명](https://docs.rs/loom/latest/loom/).
6. race는 barrier/start/finish handshake를 우선한다. fixed sleep만으로 특정 interleaving을 입증하지 않는다. 수치·fairness 모델은 전체 accepted domain의 arithmetic bounds와 경계/property test를 사용한다.

## qualification matrix

| Rail | 필요한 결과 | 미충족 처리 |
|---|---|---|
| contract/engine/runtime | debug/release focused regression, production-linked model, quota/phase conservation, cancellation/cleanup/stack/fallback 경계 | 구현 완료와 분리; NOT_QUALIFIED |
| benchmark semantics | operation/population conservation, strict invalid input, independent scheduler/loadgen oracle | 성능 수치 보고 금지 |
| Linux IAI/performance | 호환 baseline, 실제 정상 control·>5% negative, env fingerprint | macOS 결과로 대체 불가; EXTERNAL_BLOCKED/UNVERIFIED |
| static/CI | inventory required 집합, local/CI parity, scanner/Semgrep negative, actual test collection | required skip은 실패 |
| dependency/MSRV | revision 고정 advisory graph, 실제 declared MSRV 외부 consumer default/Rayon, dev toolchain 별도 | UNKNOWN을 지원 완료로 승격 금지 |
| PM (retired 2026-09-23) | 실제 생성 target 0으로 renderer와 vacuous gate 제거 | 생성 대상이 생기면 owner와 함께 재설계 |
| hosted/review/merge | exact SHA checks·required protection·승인·merge identity | 확인 안 됐으면 UNVERIFIED |
| consumer/activation | consumer repo+SHA, version/inventory/config, deployment identity, runtime 관측 | library local green으로 대체 불가 |

## receipt 최소 schema — 제안

```json
{
  "schema_version": 1,
  "ticket_id": "H16-014",
  "acceptance_ids": ["H16-014-A01"],
  "source": {
    "head": "<40-hex>", "tree": "<git-tree>",
    "immutable": false,
    "paths_digest": "<path-content-mode-symlink-including-untracked>"
  },
  "command": {"argv": ["<actual>"], "cwd": "<actual>", "env_allowlist": {}},
  "environment": {
    "toolchain": "<exact>", "lock_digest": "<sha256>",
    "features": [], "os_target": "<exact>", "runner": "<identity>"
  },
  "started_at": "<UTC>", "ended_at": "<UTC>",
  "result": {
    "status": "UNVERIFIED", "exit_code": null,
    "collected": null, "passed": null, "failed": null,
    "expected_failure": null, "operation_count": null
  },
  "artifacts": [], "limitations": []
}
```

최종 receipt schema/validator는 H16-022에서 구현한다. secret env 값은 저장하지 않는다. before/after
digest가 같아도 중간 edit-and-restore가 없었다는 증거는 아니다. hosted qualification에는 isolated
GitHub Actions checkout, exact `GITHUB_SHA`/workspace 검증과 artifact custody가 필요하다. local collection은
exact-source evidence일 뿐 `QUALIFIED`를 낼 수 없다. HEAD만 같은 dirty overlay는 동일 source가 아니다.

## 이번 계획 검증 재현

```sh
python3 docs/plans/sep-16-hardening/tickets/validate_plan.py
git diff --check
git status --short --branch
```

validator는 기존 audit 40개 파일 sha256 보존도 확인한다. git diff --check는 untracked 문서 내용을 검사하지 않으므로 validator가 새 Markdown trailing whitespace/link/ID 검사를 따로 수행한다. 최종 실행 결과는 인계 메시지에 명시한다.

## 계획 작성 시 실행 결과 — 2026-09-16

- `validate_plan.py`: PASS — 22개 티켓, 원본 40개 unique mapping 및 sha256 보존, 품질 33개, 결정 12개, acyclic final closure, Markdown 29개 local links/acceptance 검증.
- validator negative checks: duplicate primary, dependency cycle, final dependency 누락, audit digest 변조, existing path 누락, acceptance drift의 6개 in-memory mutation 모두 해당 예상 오류로 exit 1. 원본 파일을 바꾸지 않았고 mutation 해제 후 positive control exit 0.
- `git diff --check`: exit 0. 새 untracked Markdown whitespace는 별도 validator로 검사했다.
- final plan 재검토에서 의미상 proof 순환, allocation threshold validation, optional IAI graph, scanner negative case, Accepted ownership, NotDrained acceptance의 6개 보완사항을 반영했다.
- production Rust/tests/bench/hosted CI/consumer MSRV/deploy/activation: 이번 문서 작업에서 NOT RUN. 이 목록은 production qualification receipt가 아니다.
