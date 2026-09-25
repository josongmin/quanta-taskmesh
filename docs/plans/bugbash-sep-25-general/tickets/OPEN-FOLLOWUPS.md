# BG25 open follow-ups after implementation archive

The [twelve BG25 implementation tickets](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/README.md)
are archived as implementation history. `plan.json` remains `PARTIAL` because
its proof and external-adoption fields have different authorities.

| Owner | Open action | Closure evidence |
|---|---|---|
| BG25-001–003 and each deployment owner | Review D1–D9 applicability, compatibility, migration, actual ingress/planner/wire call sites, and deployed topology | Update [external adoption ledger](EXTERNAL-ADOPTION.md) with owner, exact consumer and Taskmesh source identities, selected tests, and deployment decision; mark non-applicable only with topology evidence |
| BG25-012 integrator | Keep qualification bound to each proposed final committed clean Taskmesh HEAD | `just dev`, `just verify-macos-ci`, then validate the receipt with `--expected-head`; require all applicable CI gates PASS, no required NOT_RUN/FAIL, stable HEAD/tree/path digest, and durable receipt custody. W3 commits `2a2f9bd` and `bca879a` each passed 16/16; any later commit needs its own receipt |
| CI-plan owner | Run the hosted W4 trial and decide its branch rule | W3 has selected/executed Rust and pytest denominators and clean-HEAD 16-gate receipts; the dedicated `pr-ci.yml` remains local and non-required. Hosted queue/cost result and branch-rule decision remain separate in the [verification-stage plan](../../2026-09-24-ci-verification-stages.md) |
| Release owner | Deep nightly and release qualification when explicitly requested | Complete producer denominators, Linux-only gates, compatibility adjudication, and a release decision under the [release checklist](../../../release-checklist.md) |
| Benchmark owner | Complete H28 host performance qualification **only before making a performance claim**; keep the 9-request host/simulator admission-count test as the deterministic CI proof | Same source/toolchain/features/workload/seed/warmup and quiet-host environment; independently record intended send/offered, each terminal response type, unanswered requests, post-response worker custody, and class/path sample populations. The shared host had unrelated Cargo/mutation contention during this audit, so its gate timings are not a performance baseline |

2026-09-26 execution update: clean Taskmesh `3700f3f4dd2e4b783e50ca2e3a8db9b11eba39b0`
passed all 16 CI-profile gates and its receipt validated against that exact
HEAD. Durable copy:
`/Users/songmin/.codex/artifacts/taskmesh-ss-3700f3f/macos-gates.json`
(SHA-256 `9b6776e952b9a6e359d772c8bf06e65e5b9b7b5a7b4efef54f58507f9726b1ee`).
On the later candidate tree, the new Rust test denominator completed 94
targets/632 cases; the Rayon matrix completed seven targets/33 cases; Python
collection and execution matched before the last added negative fixtures.
At that snapshot these were owner-local checks, not a final candidate-HEAD receipt. No nightly,
mutation, Linux release, or hosted run was performed by this work.

2026-09-26 W3 closeout: clean `2a2f9bd688fd249f26d1a80b0fb035669530fbad`
and documentation successor `bca879a621f7780de01127a6cc694021a54ae356`
each passed all 16 macOS CI-profile gates with exact-HEAD receipt validation.
The latter receipt is at
`/Users/songmin/.codex/artifacts/taskmesh-ss-bca879a/macos-gates.json`
(SHA-256 `8d21a9adc6fdb1b98bf6840f32857bbde282b74eaaafabf6dbc7f186d98843aa`).
The recorded clean Semantica `1bc28dc` × Taskmesh `bca879a` manual QBC
comparison passed 2/2; immutable source identities, logs, and receipt hashes
are at `/Users/songmin/.codex/artifacts/taskmesh-ss-bca879a/consumer-provenance.json`
(SHA-256 `eacaa2154480d9eee0ef45e1dddda5795f5373a414d72995a8efe95cb97da4b3`).
This is not deployment-owner acceptance. Any later Taskmesh commit must obtain
its own exact-HEAD receipt before qualification; the hosted W4 result remains open.

2026-09-26 source audit at clean `0588d26847ba575f1c059f8d86a37d8420ea5589`: H12's old wording expected a typed response from a dropped caller and new admission after the one-way drain; the active checklist now states the valid pre-drain queue order. A new `host_open_loop::terminal_caller_keeps_worker_charged_through_pre_drain_queue` fixture exercises deadline, cancel, and caller drop against the pre-drain queue and worker-custody order. It passed focused 1/1 and `just dev` 558/558 on the candidate tree; BG25-012 still needs a committed clean-HEAD CI receipt. No product-code violation was confirmed. H28's existing bounded burst comparison is a valid narrow CI fixture; its broader measurement requirement is a separate performance qualification. A05's existing input-derived ledger case is now included in `scenario-evidence.json`; no new A05 test or product patch is pending. These are evidence/qualification actions, not reopened archived implementation tickets. The 16/16 CI receipt validated at the audit HEAD does not qualify the changed tree.

The 104-row `scenario-evidence.json` is static selection provenance. Historic
receipts qualify only their recorded source. The earlier Semantica focused 2/2
run did not bind its mutable Taskmesh path dependency to a Taskmesh commit. The
later pinned-pair run binds clean Semantica `0384053` to clean Taskmesh
`76295ba` and also passes 2/2, but its authority remains
`manual_invocation_comparison_only`; deployment adoption remains open.

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
