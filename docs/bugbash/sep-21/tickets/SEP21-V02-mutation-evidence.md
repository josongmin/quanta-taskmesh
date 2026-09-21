# SEP21-V02 — Transactional mutation campaign authority

- 상태: IMPLEMENTED_UNQUALIFIED
- producer state: runner와 integrated focused campaign 검증 완료; full current-source generated
  sweep는 `NOT_RUN`, bounded `taskmesh-rayon` generated subset은 5/5 viable caught와
  5 compile-unviable를 분리해 `PASS`
- 우선순위: P1 release blocker
- 포함 finding: TM21-012, TM21-017
- 선행: V01
- write lane: `mutation-proof`

## 목적

curated mutation과 generated cargo-mutants campaign을 isolated exact-source transaction으로
실행하고, process truth·baseline·failure set·denominator를 보존한다.

## RCA

- classifier가 subprocess return code와 unrelated failure를 무시한다.
- 동일 command/env/profile의 unmutated baseline이 없다.
- live source를 제자리 변경하고 shared target을 사용한다.
- `finally` 복원은 concurrent reader와 SIGKILL을 보호하지 못한다.
- curated kill count와 generated sweep score의 authority가 과거 문구에서 섞였다.
- generated classifier가 cargo-mutants의 compile-unviable를 semantic survivor와 같은 실패로
  처리해, fail-closed 타입에 거짓 `Default`를 추가하거나 함수 전체를 제외해야만 PASS가 되는
  잘못된 유인을 만들었다.

## 확정 근거

- return code를 사용하지 않는 classifier:
  `tools/verification/run_mutations.py:237-288`.
- live source in-place mutation과 `finally` 복원:
  `tools/verification/run_mutations.py:339-369`.
- dirty tree 허용과 isolation/target lock 없는 campaign loop:
  `tools/verification/run_mutations.py:386-402,522-531`.
- per-mutant receipt에 process exit가 없는 결과 shape:
  `tools/verification/run_mutations.py:453-465`.
- generated sweep는 현재 별도 `NOT_RUN` disclosure만 가짐:
  `tools/qualification/receipt.py:359-383`.

## 목표 구조와 불변식

- campaign은 immutable source identity에서 isolated copy/worktree와 dedicated target을 만든다.
- command/env/profile fingerprint별 baseline PASS가 mutant 실행의 prerequisite다.
- control은 exit 0 + completion marker + failures 0 + tests > 0을 모두 만족한다.
- mutant kill은 expected nonzero exit + declared exact failure set/reason을 만족한다.
- unrelated failure, timeout, signal, harness error는 실패 status다.
- unviable ID는 전체 planned/executed/categorized denominator에 남기되 quality denominator에서
  제외하고 `limitations=["unviable"]`로 공개한다. caught가 0인 all-unviable campaign은 실패한다.
- curated와 generated schema/denominator/status를 절대 합산하지 않는다.

## 작업 플랜

1. `tools/verification/run_mutations.py`
   - source copy/worktree lifecycle, exclusive campaign identity, dedicated `CARGO_TARGET_DIR`.
   - preflight baseline cache와 strict exit/failure/completion classifier.
   - source-before/after digest 및 abrupt-exit cleanup protocol.
2. `tools/verification/mutations.json`
   - expected exact/co-failure set, command/env/profile identity를 명시한다.
3. 신규 `tools/verification/run_generated_mutants.py`
   - cargo-mutants raw output을 보존하고 caught/missed/unviable/timeout denominator를 구조화한다.
4. producer-owned `tools/verification/mutation-gate.json`을 추가해 V01 envelope에 필요한
   command/status/artifact contract를 선언한다. shared receipt/inventory/Justfile/CI는 직접
   수정하지 않고 V01 owner가 한 번에 등록한다.
5. `tools/verification/tests/test_run_mutations.py`와 신규 generated runner tests
   - process/isolation/classification negative matrix를 추가한다.

## 산출 artifact

- `receipt.mutations.json`
- `receipt.generated-mutations.json`
- `mutation-baseline-manifest.json`
- cargo-mutants raw JSON/log
- source-before/source-after, isolation-root, target-dir identity
- equivalent 판정의 mutant ID, source reachability, reviewer

## negative fixture

- exit 101 + parsed failure 0이 CONTROL_GREEN이 되지 않음.
- exit 0 + expected failure text가 KILLED가 되지 않음.
- expected + unrelated failure가 KILLED가 되지 않음.
- baseline red/zero-test/partial completion.
- 두 campaign과 normal cargo command 병렬 실행.
- SIGKILL 뒤 원본 digest 변화.
- generated PASS지만 raw denominator/digest 없음.
- curated count를 generated result로 복사.

## DoD

- `SEP21-V02-A01`: campaign process는 원본 source를 쓰지 않는다.
- `SEP21-V02-A02`: 각 outcome에 exit/signal/command/env/profile/test count/baseline digest가 있다.
- `SEP21-V02-A03`: concurrent/abrupt-exit test에서 source와 artifact가 오염되지 않는다.
- `SEP21-V02-A04`: current-source generated sweep을 실제 실행해 모든 survivor를
  CAUGHT/MISSED/UNVIABLE/TIMEOUT/EQUIVALENT로 보존한다.
- `SEP21-V02-A05`: equivalent는 자동 PASS가 아니며 source reachability와 reviewer가 필요하다.
- `SEP21-V02-A06`: R01은 curated 100/100을 generated score로 표현할 수 없다.

## 금지되는 임시방편

- shared checkout에 lock만 추가하고 mutant source를 계속 노출.
- output substring만으로 exit truth 대체.
- unrelated failures allow-all.
- generated survivor를 수기 표 한 줄로만 waive.
- fail-closed public type에 `Default`를 추가하거나 함수 전체를 skip해 unviable 수를 숨김.

## Producer evidence (2026-09-21)

- integrated base: `main@ba643a130dfdaf091099bb11099962198d0e5981` + producer-owned V02
  changes. Ticket 상태는 full generated sweep가 없고 실제 subset에 survivor/unviable가 있으므로
  `IMPLEMENTED_UNQUALIFIED`를 유지한다.
- producer manifest: `tools/verification/mutation-gate.json` v1,
  `sha256:c861ef1a959715f1de71f5442204b4b45aae49bd40ddb532b21f0164cec2b8bd`.
- curated inventory: 105 entries, exact failure-set policy
  `primary_plus_declared_cofailures_exact`; H02 authority 이동으로 실제 drift한 6개만 refresh했고
  current integrated source에서 anchor 105/105가 정확히 한 번 일치한다. Inventory digest는
  `00255106d5a238459d283558b420b2933822d68a8d4c7917c08c11c57371a52d`.
- runner tests: `uv run pytest -q tools/verification/tests` — 55 passed.
- lint/format: `uv run ruff check tools/verification` 및
  `uv run ruff format --check tools/verification` — passed.
- isolation negatives: exact git-visible path만 snapshot하고 ignored/unbound file은 복사하지 않음,
  symlink 자체 보존, copy 중 path-set race fail-closed, concurrent campaign root/target 분리, normal
  target sentinel 보존, isolated child `SIGKILL` 뒤 original digest 보존이 모두 passed.
- manifest safety negatives: absolute/`..`/비정규 file path, parent symlink escape, manifest의
  `CARGO_TARGET_DIR`/runner isolation env override를 모두 거부한다.
- classifier negatives: exit 0 + failure text, nonzero control, signal, timeout, zero tests, partial
  completion, expected + unrelated failure, red baseline이 모두 green으로 승격되지 않는다.
- generated parser는 cargo-mutants 27.0의 real-shape fixture와 실제 local v27 artifact에서
  `mutants.json` planned names, category txt names, `outcomes.json` executed names/summary를 모두
  exact-set 비교한다. Equal-count identity mismatch, duplicate identity, category-summary mismatch,
  red baseline, zero denominator, partial outcome, all-caught/nonzero-process가 모두 fail-closed다.
  Equivalent는 mutant ID, reachability evidence, reviewer가 있어도 자동 PASS가 아니다.
- prescribed integrated focused campaign (6 selected): baseline 4/4 `PASS`, mutation 6/6 `KILLED`,
  source before/snapshot/after
  `6192de0fdd022f0a295f142f15b5204dbf9b17da23f46e79bbd9a0322cb48a2b`, envelope problems 0.
  Full curated inventory는 `NOT_RUN`이다. 이 focused result를 full curated score로 확대하지 않는다.
- refreshed H02-authority severe campaign (6 selected): baseline 4/4 `PASS`, mutation 6/6
  `KILLED`, 같은 source identity, envelope problems 0. 중앙 authority mutation이 여러 oracle을
  깨뜨리는 경우 exact `expect_cofailures`만 선언했다.
- 첫 integrated focused 시도는 mutation 6/6이 `KILLED`였지만 V03 동시 edit로 source digest가
  변해 receipt 전체가 `FAIL`했다. 이 artifact는 non-final이며 mixed-source 증거로 재사용하지
  않았다.
- real generated subset (`taskmesh-rayon`, cargo-mutants 27.0.0, jobs=1): planned/executed/
  categorized 10/10/10, `caught=4`, `missed=1`, `unviable=5`, `timeout=0`, `equivalent=0`, process
  exit 2, signal null, timeout false, parse error null, source before/snapshot/after
  `d163fc0538a20064f541eeed2dd209f35b1de7037143ff6bccc35f654dac3328`, envelope problems 0.
  Receipt digest는 `700eba79928dc2fe66322226368280778e58fd83f1b38d710daa8fcf325837a6`이며
  semantic status는 `FAIL`이다.
- full workspace generated cargo-mutants sweep: `NOT_RUN`. Package subset/runner 검증과 full
  generated denominator를 분리하며 R01 전 실제 full sweep와 survivor/unviable 처리가 필요하다.

### V01 shared registration request

- producer id/version: `mutation-campaign` / `1`.
- envelope schema: `tools/qualification/evidence-envelope-v1.schema.json`.
- config: `tools/verification/mutation-gate.json`과 그 digest.
- curated command: `uv run python tools/verification/run_mutations.py`; required summary
  `receipt.mutations.json`, baseline manifest `mutation-baseline-manifest.json`, raw stdout/stderr logs.
- generated command: `uv run python tools/verification/run_generated_mutants.py --jobs 2`; cargo-mutants
  owns one build directory per job and the producer removes any inherited absolute
  `CARGO_TARGET_DIR`; required summary
  `receipt.generated-mutations.json`, raw `mutants.out/**`, runner stdout/stderr.
- source fields: HEAD, dirty flag, current-source tree digest, snapshot digest, original after digest,
  isolation root, dedicated target directory.
- command fields: argv, recorded environment, profile, command digest; tool fields: exact Python/Cargo/
  rustc and cargo-mutants identity as applicable.
- result rule: curated PASS requires every selected entry `KILLED` or `CONTROL_GREEN`, every command
  baseline PASS, selected/executed equality, and unchanged original source. Generated PASS requires a
  successful raw baseline, complete planned/categorized/outcome denominator, at least one caught,
  and zero missed, timeout, and equivalent outcomes. Compile-unviable identities remain mandatory,
  explicit limitations and are excluded only from the quality denominator.
- shared files requested from V01 owner only: Justfile recipe, gate inventory record, receipt linkage,
  workflow execution/upload. V02 does not modify those files.

## 검증 명령 후보

```sh
uv run pytest -q tools/verification/tests tools/qualification/tests/test_receipt.py
just mutants-critical --require-clean
# generated runner의 exact command는 ticket 구현 시 manifest에 고정
```

## Hosted CI 재감사와 oracle 정합화 (2026-09-21)

- cargo-mutants upstream contract에 맞춰 generated receipt schema를 v2로 올렸다. Upstream은
  unviable를 컴파일 불가로 정의하며 별도 조치가 필요하지 않은 결과로 취급하고, exit 0은 모든
  viable mutant가 caught됐음을 뜻한다. 근거:
  [using results](https://mutants.rs/using-results.html),
  [exit codes](https://mutants.rs/exit-codes.html),
  [algorithm](https://mutants.rs/how-it-works.html).
- 수정 producer의 실제 `taskmesh-rayon` jobs=2 campaign은 10/10 planned/executed/categorized,
  caught 5, unviable 5, missed/timeout/equivalent 0, quality 5/5, process exit 0,
  `source_unchanged=true`, `limitations=["unviable"]`, semantic PASS다. Receipt SHA-256은
  `c75c398d3eb7da6462d079f9c865a5e4aa2af61a27540fbcea74906211e8abfc`다. 이 package subset은
  producer semantics 검증일 뿐 full workspace qualification이 아니다.

- Hosted `main@1378383b63728eedd2a38d5e7f7d87c828d2f0d0`, run
  `35543694307`의 curated raw artifact는 105개 중 `KILLED=71`,
  `CONTROL_GREEN=1`, `UNRELATED_FAILURE_SET=25`, `BLOCKED_BASELINE=4`,
  `SURVIVED=4`였다. 이것은 full curated `FAIL`이며 과거 focused PASS로 대체할 수 없다.
- 25개 `UNRELATED_FAILURE_SET`는 모두 unmutated command baseline이 PASS였고,
  지정된 primary test도 실패했다. Raw per-test panic과 source operator를 대조해
  같은 변이 때문에 추가로 실패한 41개 test name을 각 inventory entry의
  `expect_cofailures`에 명시했다. 추가 실패를 무조건 허용하지 않았고
  `primary_plus_declared_cofailures_exact` 분류는 유지했다. 원인별 검토 범위:
  - Fairness/continuation: `wfq-scale-loses-precision`,
    `drr-walks-the-ring-visit-by-visit`, `drr-cursor-serves-the-wrong-class`,
    `capability-blocked-head-holds-back-every-newcomer`,
    `cross-class-newcomer-takes-the-continuation-gap`,
    `queued-behind-shed-by-overflow-policy`, `promotion-pending-is-sticky`.
    추가 test들은 같은 WFQ/DRR 연산 또는 같은 promotion-gap 판정을 직접 assert한다.
  - Ticket/memory/snapshot: `dead-ticket-survives-unwind`,
    `stage-release-skips-the-lease-touch`, `estimated-reconcile-restores-reservation`,
    `conservation-oracle-accepts-everything`. 추가 test들은 terminal retention,
    lease touch, residual reservation, snapshot conservation의 같은 source branch를 읽는다.
  - Bench oracle: `mmpp-draws-at-the-old-rate`, `reversed-trace-accepted`,
    `open-loop-histogram-double-corrected`, `contention-ops-rounded-to-thread-multiple`.
    추가 test들은 각각 동일 MMPP rate, trace time, histogram population,
    exact operation-count 계산을 assert한다.
  - Host/drain: `stack-request-charged-to-the-declared-hint`,
    `close-admission-never-refuses`, `drain-does-not-close-the-engine`,
    `drain-counts-queues-not-custody`, `drain-gives-up-before-its-budget`.
    추가 test들은 같은 physical capability resolve, admission-closed state,
    drain custody/deadline outcome을 assert한다.
  - Lease/nested wait: `lease-token-plain-release-ignores-the-lease`,
    `lease-token-nonce-not-checked`, `lease-token-minted-on-every-advance`,
    `nested-wait-cycle-is-queued-anyway`,
    `nested-wait-ignores-the-stranger-holding-a-slot`. 추가 test들은 동일 nonce/lease
    state 또는 root-vs-stranger blocker 분류를 assert한다.
- `BLOCKED_BASELINE` 중 gate inventory 3개는 isolated source에 `.git` index가 없어
  `git ls-files -z`가 128로 실패했다. Snapshot은 원본의 tracked path set만 local
  index로 재구성하고 non-ignored untracked source는 여전히 untracked로 둔다.
  나머지 하나(`long-pytest-header-loses-its-failure-reason`)는 inventory가
  삭제된 pytest test를 지목했다. 두 underscore의 긴 failure header와 assertion
  reason을 직접 검증하는 named test를 복원했다.
- 기존 survivor 네 개는 fixture drift나 확률 의존이었다: NUL class는 C01에서
  거부되어 thread-label mutant가 더는 panic을 만들지 않았고 실제 worker label
  정규화 assertion으로 교체했다. H03이 inline executor를 설치 시 거부하므로
  run-deadline anchor는 accepted-but-not-started 비차단 CPU executor의 worker-start
  budget oracle로 교체했다. 이전 ID `run-deadline-judged-from-the-timer-only`는
  H03 이후 의미가 맞지 않아 retired; 새 ID는 `run-deadline-starts-before-worker`다.
  Historical receipts의 이전 ID는 변경하지 않는다. Abandon mutant는 난수 differential sequence 대신
  64-grant continuation gap의 re-entrant waker에서 동기 postcondition을 잡는다.
  Nested-wait의 기존 `by_parent > 0` 제거는 positive pool blocker에서
  `in_use == 0 == by_parent`가 불가능해 등가였으므로 parent+stranger가 함께
  점유한 pool을 단독 root cycle로 오판하는 `>=` 변이로 교체했다.
- 변경 중 한 focused 시도는 3개 `KILLED`/1개 `WRONG_REASON`였지만
  `source_unchanged=false`라 비권위 diagnostic이다. 이후 source-bound focused
  receipt `target/sep21/v02/curated-focused-final/receipt.mutations.json`은
  5/5 exact `KILLED`, baseline 5/5 PASS, problems 0, source before/snapshot/after
  `b4fcdc9c830a9ad5fcd83e4ff17d7af73d713ef0abf37bd0e6dd5b75ef048488`이다.
  이는 선택한 5개만 증명한다. 25개 cofailure 정합화 후의 full 105 curated와
  full generated workspace sweep은 아직 `NOT_RUN`이며 release qualification이 아니다.

### CP9 hosted 105/105 denominator (run 35547086905)

- `main@a8c7d3f2282a20f93ea5429d85e42f6a87e7178c`의 hosted curated
  artifact는 `KILLED=102`, `CONTROL_GREEN=1`, `UNRELATED_FAILURE_SET=1`,
  `SURVIVED=1`, total 105, `source_unchanged=true`, 전체 `FAIL`이다.
- `drain-counts-queues-not-custody`는 primary와 2개 cofailure가 실패했지만,
  `abandoning_an_unclaimed_promotion_is_a_custody_return_the_drain_hears`는
  같은 실행에서 통과했다. 해당 test는 promoted/unclaimed 상태를 관측한 직후
  장기 drain task의 완료 여부를 읽어 scheduler timing에 의존했다. 별도의
  `drain(Duration::ZERO)`로 `inflight=1, queued=0`이 반드시 `NotDrained`인지
  동기적으로 검사하도록 semantic oracle을 보강했다. Exact cofailure 집합은
  완화하지 않았다.
- `lease-token-nonce-accessor-lies`의 constant-1 accessor는 최초 발급 nonce가
  실제로 1이라 기존 단일-token round trip을 통과했다. 두 개의 동시 lease proof에서
  accessor nonce가 다르고 각각 exact token을 재구성함을 검증하는 test를 추가하고
  그 test를 mutant의 named failure로 지정했다.
- 이 두 보완은 CP9 이후 소스 변경이다. 보완 뒤 focused/full curated와 full
  generated 결과는 CP9 `FAIL`을 closure로 승격하지 않는다. 보완 뒤 focused
  receipts `target/sep21/v02/nonce-focused-cp10b/receipt.mutations.json`과
  `target/sep21/v02/drain-focused-cp10/receipt.mutations.json`은 각각
  1/1 exact `KILLED`, baseline PASS, `source_unchanged=true`, problems 0이다.
  최초 nonce focused 실행은 기존 twin test의 실행 순서 의존으로
  `UNRELATED_FAILURE_SET`였고, 별도 governor에서 nonce 하나를 먼저 소비해
  원래 twin test가 constant-1 accessor를 항상 거부하도록 했다. Wrong-token
  test는 accessor에 의존하지 않는 zero proof를 사용한다. 이 두 focused 실행은
  최종 committed source의 full curated 결과를 대체하지 않는다.
