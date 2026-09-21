# SEP-21 execution ledger

## Campaign baseline

- branch: `hardening/sep-16`
- commit: `48aa2092572b8838561a499a0e8cac72fc5c152e`
- tree: `e16e1d776935e6f199ae5fa045c6d59db9c8a948`
- timestamp: `2026-09-21T04:12:22+09:00`
- baseline state: tracked/untracked clean immediately after commit
- remote at freeze: `origin/hardening/sep-16 = 1f04ccb245fc631507df780b97768f8d2f0e5e1a`
- baseline commit origin: created externally during campaign startup with subject `done`; it contains
  the previous 19 tracked dirty paths plus the SEP-21 audit/checklist artifacts.

The commit is a source checkpoint, not implementation or qualification proof.

## Protocol availability

- `$ss` / `$rr` named skills are not installed in the current tool surface.
- Equivalent enforced sequence: source freeze → exclusive write lane → owner-local negative tests →
  coordinator review → checkpoint commit/push → dependent lane handoff → final release receipt.

## Current wave

| packet | tickets | owner | state | dependency signal |
| --- | --- | --- | --- | --- |
| contract | C01 | `sep21_contract` | LOCALLY_VERIFIED | `C01_CONTRACT_READY` accepted |
| engine-core | E01,E02,E03,E04 | `sep21_engine` | LOCALLY_VERIFIED | `E04_RESOLVER_READY` accepted at `0fb1874` |
| proof-envelope | V01 | `sep21_v01` | IMPLEMENTED_UNQUALIFIED | integrated at `9db7903`; hosted receipt NOT_RUN |
| host | H01,H03,H02 | `sep21_host` | LOCALLY_VERIFIED | `HOST_PACKET_READY` accepted at `ba643a1` |
| mutation | V02 | `sep21_v02` | IMPLEMENTED_UNQUALIFIED | producer checkpoint `df08a59`; full generated sweep remains NOT_RUN |
| concurrency | V03 | `sep21_v03` | LOCALLY_VERIFIED | producer checkpoint `3969063`; hosted exact-source replay remains NOT_RUN |
| release | R01 | `sep21_release_r01` | IMPLEMENTING_UNQUALIFIED | release-only implementation; final source and version decision remain open |

## Checkpoints

| checkpoint | source | evidence | remote state |
| --- | --- | --- | --- |
| CP0 audit baseline | `48aa209` | strict plan validator was PASS on pre-commit equivalent source; baseline metadata re-frozen after commit | pending push |
| CP1 C01 contract | `eab2ceb` | contract tests 56/56, adversarial 11/11, clippy `-D warnings`, structure validator and diff check PASS | pushed to `origin/main` |
| CP2 V01 phase A | `32189ab` | full V01 suite 130 PASS; independent targeted 86 PASS; ruff, 24/24 inventory parity, structure and diff check PASS; real Linux IAI NOT_RUN | pushed to `origin/main`; phase B open |
| CP3 E01-E04 engine core | `0fb1874` | engine default 224/224, Loom 5/5, Shuttle 7/7 at 10,000 schedules/model, clippy `-D warnings`, structure validator and diff check PASS; capacity conservation zero residuals | pushed to `origin/main` |
| CP4 H01-H03-H02 host | `ba643a1` | contract 57/57, taskmesh default 160/160, rayon 160/160, adapter 4/4, consumer MSRV default/rayon on Rust 1.81, four clippy `-D warnings` configurations PASS; coordinator focused rerun 44/44 plus rayon clippy PASS | pushed to `origin/main` |
| CP5 V02 mutation producer | `df08a59` | producer tests 55/55 PASS, final focused curated 6/6 KILLED; generated `taskmesh-rayon` subset 4 caught, 1 missed, 5 unviable; full curated and full workspace generated NOT_RUN | pushed to `origin/main`; V02 remains IMPLEMENTED_UNQUALIFIED |
| CP6 V03 fuzz/model producer | `3969063` | producer tests 18/18 PASS; short local fuzz admission 6,371, policy 6, wire 23,245 runs with semantic checkpoints; model evidence 12/12, Loom 1,611 permutations, Shuttle 60,000 schedules and separate replay generation/verification PASS | pushed to `origin/main`; hosted exact-source run NOT_RUN |
| CP7 V01 integration and proof stability | `9db7903` (tree `85af6fdbd0044a65fa21461c5c5444a709453829`) | V01 prescribed 256/256, focused receipt/inventory 194/194, producer Python 74/74, full `just py-test` 469/469, 105/105 curated anchors, final focused 1/1 KILLED with before/snapshot/after digest `84cd8cb8c66a1d6c30f8e8c95b5f3b846ccfdf05e8d44805db55950eff31622e`; engine full suite and clippy PASS; H03 default/rayon 2/2 and 200/200 flaky-test repetitions; `just bench-gate` 8.0 PASS; ruff, fmt, inventory 26/26/26, structure and diff checks PASS. This is dirty-overlay local evidence, not a clean hosted receipt. | pushed to `origin/main`; V01 remains IMPLEMENTED_UNQUALIFIED; full curated/generated, Linux IAI and hosted qualification NOT_RUN |
| CP8 hosted qualification attempt | `1378383` (tree `d081997f`) | GitHub Actions run `35543694307` concluded failure and its qualification job returned `NOT_QUALIFIED`. Passing producer jobs included coverage, MSRV, Loom, Shuttle, model replay and TSan. Failed jobs included fast gate (Rust 1.98 Clippy), bench matrix (stale bench API), fuzz (cargo-fuzz selected musl target), generated mutation (bench baseline did not compile), and curated mutation (4 SURVIVED, 25 UNRELATED_FAILURE_SET, 4 BLOCKED_BASELINE among 105). The generated 0-count result is not a mutation-quality score. | pushed to `origin/main`; hosted qualification FAILED, not closure |
| CP9 integrated repair and R01 producer | `a8c7d3f` | Local bench compile, doc examples 3/3, Python tools 516/516, R01 tests 46/46, fmt, Ruff, inventory, architecture and dependency audit PASS; local Rust 1.98 Clippy all-target phase PASS but full command was interrupted after later source changes. Hosted run `35547086905`: fuzz, MSRV, Loom, Shuttle, model replay, TSan, coverage PASS; fast gate failed at cargo-deny 0.20.2 CLI drift, bench-smoke failed on a malformed child fixture, curated 105 yielded 102 KILLED + 1 CONTROL_GREEN + 1 UNRELATED_FAILURE_SET + 1 SURVIVED. Generated sweep remained in progress at this ledger edit. | pushed to `origin/main`; hosted qualification FAILED, not closure |
| CP10 hosted oracle and CLI repair | `a3e3b3c` | Local focused nonce/drain mutants each KILLED with clean baseline and unchanged source; lease tests 8/8, bench-smoke compile, cargo-deny 0.19.7 and inventory passed. Hosted run `35548603010`: MSRV, Loom, Shuttle, TSan, model replay, fuzz, coverage and bench-smoke passed; fast gate failed on 21 Semgrep findings. Curated 105 yielded 103 KILLED + 1 CONTROL_GREEN + 1 UNRELATED_FAILURE_SET: the unique-nonce mutant also failed the newly added two-live-proofs test, which was absent from its exact expected-failure set. Generated sweep was still in progress. Separate `bench-gates` run `35548603153` failed: IAI runner rejected `SAVE_SUMMARY=yes` (requires `json` or `pretty-json`), and Criterion tried to read an incomplete cached `base/sample.json`. | pushed to `origin/main`; hosted qualification FAILED, not closure |
| CP11 static and benchmark producer repair | `d7c745b` (tree `a35206a`) | Local full Rust workspace tests, three Clippy lanes, deny, Semgrep 0 findings, architecture, Python 516/516, allocation 8.0, inventory 27/26/27 and fuzz-check passed; exact nonce mutant KILLED with 2/2 expected failures and unchanged source. The exact-source 23-finding witness producer passed 23/23 with unchanged digest. Hosted CI `35550137221` passed MSRV, matrix, Loom, Shuttle, model replay, TSan, fuzz and coverage, but fast gate found two CI-context test defects: a synthetic local V02 fixture inherited hosted identity, and shallow checkout omitted the immutable `0.2.0` baseline. Bench run `35550137226` attempt 1 produced 3/3 valid IAI summaries as `BASELINE_CREATED`; wall-clock publication measured correctly but failed on three 1.54–1.61x alerts. Same-SHA attempt 2 completed success: IAI `QUALIFIED` with 3/3 comparisons and wall-clock 472ns/111ns/686ns for the three alerted paths, below the prior 489ns/113ns/702ns values. | pushed to `origin/main`; benchmark rail qualified on same-SHA repeat, fast gate still FAILED |

The earlier H01-only detached checkpoint (focused 21/21, default/rayon 154/154) is superseded by
the integrated CP4 host transaction. H01, H03 and H02 are not treated as independently mergeable
patches.

CP0 local checkpoint commit is `3da26491997c81435c969bd05b8a20438e360f99`. It was
fast-forwarded into local `main` and observed at `origin/main`; direct push to
`origin/hardening/sep-16` initially failed with HTTP 403. CP1 push to `origin/main` subsequently
succeeded. `origin/hardening/sep-16` remains a separate stale remote ref and is not treated as closure.

Future entries must include exact commit/tree, changed paths, owner-local commands/counts, negative fixtures,
unexecuted items and dependency signals. A commit or push alone is not closure.

## Open integration boundaries (not release evidence)

- V01 phase B integrated locally at CP7. The independent audit rejected malformed nested JSON
  without exceptions in 748 mutations, duplicate model/fuzz/raw-artifact rows, and missing hosted
  prerequisite/provenance bindings. No clean hosted qualification receipt exists yet; CP7's focused
  curated result is one mutant, not a full inventory score.
- CP8 failures are being repaired against the producer/semantic oracle, not waived. Rust 1.98
  exposed a private-module visibility/doc lint; `taskmesh-bench` retained the old two-argument
  `child_of` API and boolean `reconcile_memory` assumptions. The generated-mutation raw baseline
  log records those bench compile errors. The fuzz job installed a musl `cargo-fuzz` executable
  on a GNU host and selected musl for ASan; the runner now explicitly passes the nightly compiler
  host target. A one-second local smoke executed all three targets but its evidence correctly
  rejected concurrent source edits (`paths_digest`); it is not a PASS. The curated campaign has
  stale targets and exact-failure sets under active review; no full curated PASS is claimed.
- CP10 follow-up addresses the observed failure surfaces: reasoned legacy alias lint attributes,
  assertions instead of discarded test outcomes, and an exact host-time exception in Semgrep;
  the two model replay witness lines retain explicit scoped suppressions because the replay
  producer consumes them. The nonce mutant's cofailure set now includes the independent
  two-live-proofs oracle. IAI requests the runner's documented `json` summary format. Both
  wall-clock trend jobs use a fresh Criterion home so an incomplete build-cache baseline
  cannot contaminate a new measurement. These edits require a new exact-source hosted run;
  no prior run is retroactively qualified.
- CP11 proved all 23 finding witnesses on a clean exact source, but that proof is checkpoint-local.
  The fast-gate failures require a local-fixture environment boundary and full-history checkout for
  immutable-baseline consumers. The first valid IAI run established a baseline only. The wall-clock
  first wall-clock alert was contradicted by a same-SHA repeat while the IAI repeat produced real
  comparison proof. The noisy first point remains reported; only attempt 2 is the CP11 benchmark
  qualification evidence.
- The committed pre-SEP-21 allocation baseline was 3 allocations/admit→release. The integrated
  source initially measured 16; borrowed validation plus an exact-operation permit index reduced
  it to 8. Three current-source 200,000-op probe runs each measured 8.000 with self-check 1000/1000
  and zero residual inflight. `tools/bench/perf-gate.json` now explicitly accepts the +5 regression
  with no cushion; parser negatives were 33 PASS (2 slow tests deselected), and the real
  `just bench-gate` passed at 8.0. Linux IAI comparison remains separate proof. This is not a
  performance improvement claim.
- The only identified `0.2.0` release commit is `39bee682d7daa1efaf1c10993ba6221fd0a90871`;
  there is no release tag. R01 must freeze an immutable baseline SHA and a final candidate SHA.
  Ordinary hosted qualification of a PR merge SHA is evidence only for that SHA, not a release
  verdict for a different final source.
- `docs/release-checklist.md` still describes the generated mutation surface as `NOT_RUN` and
  excluded, while V01's candidate ordinary required set requires a full generated result. V02's
  candidate PASS rule additionally rejects missed, unviable, timeout and equivalent outcomes;
  the observed `taskmesh-rayon` subset had one missed and five unviable. The full workspace sweep
  is `NOT_RUN`. R01 must reconcile policy and documentation without promoting `REPORTED` to
  mutation-quality PASS or silently waiving a non-caught outcome.
