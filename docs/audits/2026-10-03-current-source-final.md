# 2026-10-03 current-source audit

- **Verdict:** Taskmesh remediation passed local 16-gate CI at
  `f2808d379b743fc5eaa7e295987c906dd7bc2957` and the hosted required
  check at its synthetic merge SHA `85907fc3fdef2c13962daf186f7f0b6ab2a08d5e`.
  Measured performance, release qualification and external deployment
  acceptance are **OPEN**. A later commit requires its own exact-source checks.
- **Initial source under review:** `main@c4bcb2f213c66aac8adc7d3493c5a39917a29e00`
  (tree `c99caff9d5b32f69ca6fbde92a2ff14dd19a8b88`) plus uncommitted
  documentation, test and product edits. The baseline HEAD's hosted result is
  not evidence for these working-tree bytes.
- **Consumer read-only observation:** Semantica
  `a961cf26842e77fc6231f16b057626c69cb7897d` had unrelated dirty paths.
  Its manifest and lockfile still pin Taskmesh `c4bcb2f`, and its blocking
  adapter still calls `run_blocking_with` with `with_absolute_deadline`.
  Semantica was not edited.

## Current findings and disposition

| Area | Source finding / result | Remaining boundary |
|---|---|---|
| Required PR check | GitHub `main` protection has strict `required 16-gate verdict`; `pr-ci.yml` is active for PR/main. The [PR #9 run](https://github.com/josongmin/quanta-taskmesh/actions/runs/37105889890) passed 16/16 on synthetic merge `85907fc` for head `f2808d3`. | Every later head/merge SHA needs its own result. |
| Synchronous deadline | `CompleteBy` still rejects blocking/CPU by contract. Taskmesh now has `run_blocking_response_by` and `run_cpu_response_by`: absolute acquisition and caller-response bound, pre-start refusal, and retained lease for a started worker. | Semantica still calls the old contract. Fixed-pair consumer and D1–D9 deployment acceptance are deferred and OPEN. |
| Measured admission | `host_admission.py` now checks each measured attempt's reconstructed producer lag against its hash-bound rate policy before `MEASURED_SERIES_ADMITTED`. | B00 budgets, quiet-host repeated series and performance verdict are NOT_RUN. |
| SDK / Rayon | Facade reexports plan error types; Rayon panic constructors were removed and `RayonBuildError` reexported. | Removing public methods is a Rust source break; final 0.2.0-to-0.3.0 semver adjudication remains OPEN. |
| Production `expect` | Default-feature uses were classified by their validated/private preconditions. No caller-controlled panic counterexample was established. Test-only and `test-util` cases are separate; Rayon panic wrappers are removed. | A discovered bypass must get a typed rejection and negative regression; no blanket conversion is justified by count alone. |
| Module layout | Host acquisition, synchronous dispatch and detached settlement; engine memory, observation, validation and state transitions; fairness tests were split without changing crate boundaries or inventory. Parent runtime/governor/state files are 860/903/655 lines. | Local and hosted CI passed at the source identities above; future source changes requalify. |
| Rust toolchain | Hosted rolling stable Rust deprecated `AtomicU64::fetch_update` under `-D warnings`; the governor now uses one checked `compare_exchange_weak` loop on Rust 1.81. The next hosted run exposed Rust 1.99's style-only `assert_is_empty` lint. CI now selects Rust 1.99.0 explicitly, allows that assertion style in the existing lint policy, and rewrites the separate fuzz harness assertion. | Local Rust 1.99.0 and hosted 16-gate checks passed for `f2808d3`; later compiler upgrades need a reviewed source change. |
| Nightly | Full `ci.yml`, `bench.yml` and `release.yml` remain manually triggered and disabled on GitHub; no cron is present. | A mutation-bearing schedule needs explicit authorization under `AGENTS.md`, runner budget and receipt authority. A bounded scheduled CI subset would have a different denominator. |
| Old receipts | The three ignored root receipts naming `cc5b256` were moved byte-for-byte to `target/verification/historical/cc5b256704a1826e7dde36e2c4b127c6cba8aabf/`. | They remain historical diagnostics; validate any cited receipt with `--expected-head`. |

## Documentation custody

- Historical Sep-16, BG25, benchmark, CI-stage and SEP-27 narrative plans/RFCs
  were removed from the active documentation tree. Completed decisions are
  compressed in [ADR 0003–0007](../adr/README.md) and measurement boundaries in
  [ADR 9000](../adr/9000-benchmark-strategy.md).
- The 104-case mapping and historical receipts live under
  [`docs/evidence`](../evidence/sep25/README.md); the
  [operating kernel](../ssot/README.md) is a navigation index, not a new API
  or release authority. The sole active work order is the
  [current remediation plan](../plans/2026-10-03-current-source-remediation.md).
- Sep-21 `plan.json`/tickets/packets remain at their source-bound paths because
  release receipt, finding proof and adjudication tooling consume them. They
  are release evidence inputs, not an active implementation queue. Moving them
  requires a versioned manifest migration and new exact-source proof.

## Initial dirty-source verification

| Command / scope | Result |
|---|---|
| `just dev` | PASS: format, production Clippy, 582 Rust tests, Semgrep 186 files/0 findings, architecture, ticket/scenario mapping and gate inventory. |
| `cargo test --locked -p taskmesh --test hardening_deadline_custody` | 19/19 PASS, including expired/queued/running/held-CPU/caller-drop/dedicated-stack response-bound cases. |
| `uv run pytest tools/bench/tests/test_host_admission.py tools/bench/tests/test_host_control_assess.py tools/bench/tests/test_host_compare.py -q` and relevant Ruff | 77/77 PASS and Ruff PASS. |
| `just consumer-msrv` | Default and Rayon PASS on declared Rust 1.81. |
| `just test-rayon`, `just doctest`, `just rustdoc` | PASS. |
| Markdown local-link resolution and `git diff --check` | PASS. |

Those commands ran on the initial dirty audit source, so they did not issue a
clean-source or release receipt. A later clean candidate
`aa116aa59889f01bc997d77176ef1d4df75a2693` passed all 16 local macOS CI
gates and its receipt validated against that exact HEAD. Its 0.2.0 baseline
semver audit was `REPORTED` with findings in all four public crates, including
the intentional Rayon method removals. The [first PR run](https://github.com/josongmin/quanta-taskmesh/actions/runs/37102825665)
failed Clippy on GitHub's newer stable Rust because `fetch_update` became
deprecated. The [second PR run](https://github.com/josongmin/quanta-taskmesh/actions/runs/37103771725)
then failed on Rust 1.99's new assertion-style lint. After the atomic sequence,
toolchain and fuzz harness repairs, clean `f2808d3` passed local macOS CI 16/16
under Rust 1.99.0; the saved receipt validated with `--expected-head` set to
its full SHA. The [final PR run](https://github.com/josongmin/quanta-taskmesh/actions/runs/37105889890)
passed hosted Linux CI 16/16 at synthetic merge `85907fc`. Its uploaded receipt
has `qualified=true`, no required FAIL/NOT_RUN, matching before/after source,
and a tree identical to `f2808d3`. The semver producer was rerun on that clean
head and remained `REPORTED` with findings in all four public crates; human
Rust API/wire/behavior adjudication is OPEN. Mutation/nightly, Linux release,
measured performance and Semantica deployment acceptance remain **NOT_RUN**.
