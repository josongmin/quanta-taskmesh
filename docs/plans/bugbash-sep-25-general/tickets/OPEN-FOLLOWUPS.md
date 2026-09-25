# BG25 open follow-ups after implementation archive

The [twelve BG25 implementation tickets](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/README.md)
are archived as implementation history. `plan.json` remains `PARTIAL` because
its proof and external-adoption fields have different authorities.

| Owner | Open action | Closure evidence |
|---|---|---|
| BG25-001–003 and each deployment owner | Review D1–D9 applicability, compatibility, migration, actual ingress/planner/wire call sites, and deployed topology | Update [external adoption ledger](EXTERNAL-ADOPTION.md) with owner, exact consumer and Taskmesh source identities, selected tests, and deployment decision; mark non-applicable only with topology evidence |
| BG25-012 integrator | Qualify each proposed final committed clean Taskmesh HEAD | `just dev`, `just verify-macos-ci`, then validate the receipt with `--expected-head`; require all applicable CI gates PASS, no required NOT_RUN/FAIL, stable HEAD/tree/path digest, and durable receipt custody |
| CI-plan owner | Finish W3 execution denominator and decide W4 hosted adoption | Update the [verification-stage plan](../../2026-09-24-ci-verification-stages.md) with source-backed implementation and separate exact-source proof |
| Release owner | Deep nightly and release qualification when explicitly requested | Complete producer denominators, Linux-only gates, compatibility adjudication, and a release decision under the [release checklist](../../../release-checklist.md) |
| Benchmark owner | Make a performance claim only after quiet-host comparison | Same source/toolchain/features/workload/seed/warmup and explicit response/worker denominators |

The 104-row `scenario-evidence.json` is static selection provenance. Historic
receipts qualify only their recorded source. The Semantica focused 2/2 run is
a manual comparison whose receipt does not bind its mutable Taskmesh path
dependency to a Taskmesh commit; deployment adoption remains open.

After the implementation archive, H31's combined external lease preemption,
host non-execution, second admission, and drain boundary was added at `6c45345`.
`taskmesh --lib` passed 24/24 at that source; the H31 scenario now selects
`external_preemption_blocks_host_dispatch_and_drain_until_token_release_v1`
as its primary case. The archived review's H31 gap describes its earlier
source state. H02, H17, H24, D03, D04, and D19 also received focused evidence
updates after their earlier exact-HEAD receipts. A clean CI-profile receipt at
`76295ba956fe2ef867e405cb3b204f1ef2b4185b` passed all 16 required gates
with no required `NOT_RUN` or `FAIL`; `just dev` passed 557/557 and `py-test`
passed 722/722. The durable receipt at
`/Users/songmin/.codex/artifacts/taskmesh-adr-archive-76295ba/macos-gates.json`
has SHA-256 `9b1270d3695e7e64fffcd2165b80d17749e088dc390081ab3d2792a639c49e2d`
and was validated against that exact HEAD. This proof is source-bound: a later
candidate needs its own receipt. The plan's static `RECEIPT_REQUIRED` marker and
external-adoption work remain open.
