# BG25 open follow-ups after implementation archive

The [twelve BG25 implementation tickets](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/README.md)
are archived as implementation history. `plan.json` remains `PARTIAL` because
its proof and external-adoption fields have different authorities.

| Owner | Open action | Closure evidence |
|---|---|---|
| BG25-001–003 and each deployment owner | Review D1–D9 applicability, compatibility, migration, actual ingress/planner/wire call sites, and deployed topology | Update [external adoption ledger](EXTERNAL-ADOPTION.md) with owner, exact consumer and Taskmesh source identities, selected tests, and deployment decision; mark non-applicable only with topology evidence |
| BG25-012 integrator | Qualify the final committed clean Taskmesh HEAD after this relocation | `just dev`, `just verify-macos-ci`, then validate the receipt with `--expected-head`; require all applicable CI gates PASS, no required NOT_RUN/FAIL, stable HEAD/tree/path digest, and durable receipt custody |
| CI-plan owner | Finish W3 execution denominator and decide W4 hosted adoption | Update the [verification-stage plan](../../2026-09-24-ci-verification-stages.md) with source-backed implementation and separate exact-source proof |
| Release owner | Deep nightly and release qualification when explicitly requested | Complete producer denominators, Linux-only gates, compatibility adjudication, and a release decision under the [release checklist](../../../release-checklist.md) |
| Benchmark owner | Make a performance claim only after quiet-host comparison | Same source/toolchain/features/workload/seed/warmup and explicit response/worker denominators |

The 104-row `scenario-evidence.json` is static selection provenance. Historic
receipts qualify only their recorded source. The Semantica focused 2/2 run is
a manual comparison whose receipt does not bind its mutable Taskmesh path
dependency to a Taskmesh commit; deployment adoption remains open.
