# BG25 open follow-ups after implementation archive

The [twelve BG25 implementation tickets](../../../adr/0007-sep-25-implementation-closure.md)
are compressed into the accepted implementation record. `plan.json` remains `PARTIAL` because
its proof and external-adoption fields have different authorities.

| Owner | Open action | Closure evidence |
|---|---|---|
| BG25-001–003 and each deployment owner | Review D1–D9 applicability, compatibility, migration, actual ingress/planner/wire call sites, and deployed topology | Update [external adoption ledger](EXTERNAL-ADOPTION.md) with owner, exact consumer and Taskmesh source identities, selected tests, and deployment decision; mark non-applicable only with topology evidence |
| BG25-012 integrator | Qualify each proposed final committed clean Taskmesh HEAD | `just dev`, `just verify-macos-ci`, then validate the receipt with `--expected-head`; require all applicable CI gates PASS, no required NOT_RUN/FAIL, stable HEAD/tree/path digest, and durable receipt custody |
| CI-plan owner | Decide W4 required-check adoption and configure the branch rule | The [verification-stage plan](../../2026-09-24-ci-verification-stages.md) records exact-HEAD PR and main-push CI trials; the aggregate check remains non-required until the branch rule is set |
| Release owner | Deep nightly and release qualification when explicitly requested | Complete producer denominators, Linux-only gates, compatibility adjudication, and a release decision under the [release checklist](../../../release-checklist.md) |
| Benchmark owner | Complete H28 host performance qualification **only before making a performance claim**; keep the 9-request host/simulator admission-count test as the deterministic CI proof | Same source/toolchain/features/workload/seed/warmup and quiet-host environment; independently record intended send/offered, each terminal response type, unanswered requests, post-response worker custody, and class/path sample populations. Do not compare simulator counts for terminal types it does not model or label simulator admission wait as host latency |

The merge of PR #3 and the committed H12 follow-up reached remote `main` at
`f26f2fff91b1424451d269dc5abca7e5a7b36978`. Its macOS CI profile passed
16/16 and the automatic [main-push run](https://github.com/josongmin/quanta-taskmesh/actions/runs/36162385719)
passed 16/16 at the same HEAD. The downloaded Linux receipt validated against
that source (Rust 632/632; pytest 748/748). This source is superseded by any
subsequent integration commit; repeat exact-HEAD qualification after changes.

Clean `2a2f9bd` and `bca879a` each passed a separate macOS 16/16 CI-profile
receipt before integration. The clean Semantica `1bc28dc` × Taskmesh
`bca879a` manual consumer comparison passed 2/2 with immutable source and
artifact identities in the [adoption ledger](EXTERNAL-ADOPTION.md). These
historical receipts do not qualify the later merged source or establish
deployment-owner acceptance.

2026-09-26 source audit at clean `0588d26847ba575f1c059f8d86a37d8420ea5589`: H12's old wording expected a typed response from a dropped caller and new admission after the one-way drain; the active checklist now states the valid pre-drain queue order. A new `host_open_loop::terminal_caller_keeps_worker_charged_through_pre_drain_queue` fixture exercises deadline, cancel, and caller drop against the pre-drain queue and worker-custody order. It passed focused 1/1 and `just dev` 558/558 on the candidate tree; BG25-012 still needs a committed clean-HEAD CI receipt. No product-code violation was confirmed. H28's existing bounded burst comparison is a valid narrow CI fixture; its broader measurement requirement is a separate performance qualification. A05's existing input-derived ledger case is now included in `scenario-evidence.json`; no new A05 test or product patch is pending. These are evidence/qualification actions, not reopened archived implementation tickets. The 16/16 CI receipt validated at the audit HEAD does not qualify the changed tree.

The 104-row `scenario-evidence.json` is static selection provenance. Historic
receipts qualify only their recorded source. The earlier Semantica focused 2/2
run did not bind its mutable Taskmesh path dependency to a Taskmesh commit. The
later pinned-pair run binds clean Semantica `0384053` to clean Taskmesh
`76295ba` and also passes 2/2, but its authority remains
`manual_invocation_comparison_only`; deployment adoption remains open.
The user identified Semantica as the consumer for this audit on 2026-09-26;
the deployed Semantica binary/source SHA and D1–D9 applicability still need
deployment evidence. See the [external ledger](EXTERNAL-ADOPTION.md).

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
