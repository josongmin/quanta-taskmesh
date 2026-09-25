# Plan review and implementation status

## Verdict

The 104-row static mapping validates on the current tree; this is structural evidence, not an execution receipt. The in-repository semantic audit has bounded compound fixtures for overload, capacity, policy, concurrency, child lifetime, and host/simulator accounting. Six overbroad oracles were narrowed and B27's destructor-panic path was added at `605070d`. Clean committed Taskmesh `a23dcda37939cd1956e72dda962c137303b1ed1d` passed `just dev` (554/554) and a validated macOS CI-profile receipt (16/16 required gates, required NOT_RUN/FAIL 0, unchanged HEAD/tree/path digest). That receipt qualifies only `a23dcda`; any later document commit needs its own exact-HEAD receipt. D1–D9 deployment review and external ingress/planner/wire adoption remain OPEN. Full nightly modelcheck, TSan, fuzz, coverage, IAI, mutation, and release qualification have not run. Semantica's clean 2/2 focused consumer comparison was rerun against Taskmesh `da5356b`; its QBC invocation has manual comparison authority, not owner/deployment qualification.

## 2026-09-25 final residual audit

| Priority | Owner | Remaining action and DoD | Evidence boundary |
|---|---|---|---|
| Required for this updated tree | BG25-012 integrator | On the final clean HEAD run `just dev`, then `just verify-macos-ci`, and validate the receipt with `--expected-head "$(git rev-parse HEAD)"`. Require 16 applicable required gates PASS, no required NOT_RUN/FAIL, unchanged HEAD/tree/path digest, and durable receipt custody. | The validated `a23dcda` receipt predates this tracked audit update. Its PASS cannot qualify a later tree. |
| Required for deployment adoption | Product/consumer owner with BG25-001–003 | Review D1–D9 against each deployed Taskmesh consumer. Identify the exact bytes ingress, child planner, classifier, serialized wire consumer, and deadline/executor assumptions or record a source-backed N/A for each absent boundary. Run consumer fixtures against final Taskmesh source and obtain owner/deployment qualification. | Semantica's scoped typed-builder path and clean focused 2/2 test do not prove deployment approval or exhaust all consumers. |
| Required only for nightly/release claim | BG25-009/012 and release owner | On explicit high-cost authorization, run all registered source-bound modelcheck, TSan, fuzz, coverage, IAI, and mutation producers as applicable; validate complete raw receipts and release checklist/semver/adjudication separately. | Bounded deterministic CI and focused Shuttle replay do not cover the full nightly state space. |
| Required only for performance claim | BG25-011 benchmark owner | Capture a quiet-host baseline with same source/toolchain/features/workload/seed/warmup and exact response/worker denominators before claiming latency or throughput improvement/regression. | Current host/simulator correctness tests and `bench-smoke` are not performance qualification. |

No additional reachable production defect is established by this audit. `scenario-evidence.json` remains a static target/case map; CI PASS shows selected cases ran, while external product semantics and exhaustive interleavings retain the boundaries above. Hosted GitHub CI is intentionally disabled by the repository's local-first policy, so its absence is not counted as a regression.

## Classification

| Class | Tickets | Source-change rule |
|---|---|---|
| Contract decision | BG25-001, 003, 007 | Documentation/fixtures first; production behavior changes only after compatibility decision. |
| Resolved structural defect | BG25-005 H30/B25/H27 | `bf03efc`, `3e57f1a`, and `0b608c3` freeze executor authority and its runtime prerequisite, preflight Tokio-backed dispatch including explicitly installed built-ins, and prove aggregate shared-executor bounds. |
| Resolved deadline defect | BG25-006 H34 | `74a18f7` enforces `CompleteBy` at caller response while retaining worker custody through teardown. |
| Resolved contract mismatch | BG25-003 D17 | `1c7b8ac` aligns closed-admission rustdoc with actual preflight precedence and adds an exact no-side-effect fixture. |
| Strict ingress | BG25-002 | Additive bounded strict task/config entrypoints preserve raw DTO compatibility. External deployment adoption remains an ownership fact outside this repository. |
| Authority migration | BG25-004 B28 | Governor-bound opaque permit/ticket handles reject foreign same-sequence operations and fail closed on exhaustion. |
| Proof-first | BG25-008, 009, 010 | Independent admission, queue-history, and memory-ledger oracles found no additional production mismatch. |
| Measurement/evidence | BG25-011, 012 | Host and simulator accounting have deterministic fixtures. The validated 104-row manifest is a static candidate map; execution status comes from a separate exact-source receipt. |

## SOLID and minimality checks

- **Single responsibility:** contract, ingress, identity, executor, custody, child lifetime, admission, queueing, memory, load, and proof integration have separate owners.
- **Open/closed compatibility:** strict ingress is additive; raw DTO compatibility is not silently tightened.
- **Substitutability:** `CpuExecutor` implementations are judged against one frozen descriptor and the same spawn/custody contract.
- **Interface segregation:** raw DTO, validated plan, executor port, planner membership, and consumer wire handling are not collapsed into one API.
- **Dependency inversion:** engine remains runtime-agnostic; Tokio/Rayon behavior stays behind ports/adapters.

## Explicit anti-overengineering decisions

- No scheduler rewrite, dynamic borrowing, distributed coordinator, global epoch subsystem, universal structured-concurrency runtime, or new executor-specific pools.
- No tolerant wire envelope unless a real consumer requires one.
- No engine parent-plan registry unless product requirements move membership authority into Taskmesh.
- No full Cartesian test matrix; use representative pairs plus independent model/property coverage.
- No long campaigns in `dev` or ordinary CI.
- No production patch for admission/fairness/memory until a deterministic counterexample fails.

## Holes checked

- All 104 audit IDs are present; all 53 P/G IDs map exactly once and BG25-012 retains all 51 K rows as baseline evidence.
- Every implementation ticket has evidence, files, work plan, per-scenario DoD, verification, and stop conditions.
- BG25-012 depends on every implementation ticket.
- Old `sep-25-engine-coverage` is marked superseded rather than deleted.
- Existing K fixtures remain baselines; tickets target missing combinations instead of duplicating them.
- Historical receipts remain source-bound and are never upgraded to later commits. The final receipt is generated only after all tracked changes are committed.

## Current qualification boundary

- BG25-008: H01 covers four host dispatch paths and reject-before-work; B05 isolates all four capacity dimensions; H07 combines primary/scavenger/fallback/drop policy; H25 releases four blockers independently; H32 preserves measured cross-class memory pressure after class release; D01 distinguishes zero/one/exact/+1 capacity meanings.
- BG25-009: H04's bounded combined fixture starts release, claim, timeout abandon, cancel abandon, and sweep behind one barrier and checks quiescent ownership/accounting; the finite-history and host race-storm cases remain supporting evidence. This does not exhaust every internal interleaving. H06 links independent DRR/WFQ/cancellation/cross-pool references. H13 proves both drain futures reached `Pending` before direct release; H14 covers reentrant snapshot/release and callback panic outside the lock. H16 uses a bounded two-thread input ledger. Its focused Shuttle fixture saves and replays one real Governor admission ordering under an intentionally false assertion; it proves replay capability, not a product defect or the full model set.
- BG25-003/004/005/006/007/010: compound wire, identity, deadline, stack, child-lifetime, and snapshot claims use explicit supporting cases. H18 additionally depends on the `test-rayon` exact selectors and the separate default+rayon `consumer-msrv` gate.
- BG25-011: H28 compares host and simulator offered/completed/rejected/max-queue counts under the same finite burst without treating virtual wait as host latency. Quiet-host latency and multi-class performance baselines remain separate performance qualification, not correctness gaps.
- `MAPPED` remains a static source/case relationship. Execution PASS comes only from the exact-source receipt.
- The library strict ingress exists; deployment owners must explicitly call it at their untrusted byte boundary.
- Parent-plan stage membership and detached structured concurrency remain outside the engine contract by D4 and the root-child lifetime fixtures.
- Nightly modelcheck, TSan, fuzz, coverage, IAI, and mutation are NOT_RUN pending explicit authorization.

## Remaining work order

1. BG25-012: validate the external macOS CI receipt against the current clean HEAD; reissue it after any tracked change.
2. BG25-001–003 and deployment owners: decide D1–D9 applicability for each real consumer and obtain owner/deployment evidence. The Semantica focused 2/2 comparison is a lead, not deployment qualification.
3. BG25-009/012 and release owner: run the full nightly/release producers only when that qualification is requested.
4. BG25-011: measure a quiet-host baseline only when making a performance claim.

## External adoption boundary

The repository library implementation is complete for accepted contracts. Deployment owners still choose their bytes ingress call site, validate parent-plan membership before submission, and negotiate downstream wire versions; see [EXTERNAL-ADOPTION.md](EXTERNAL-ADOPTION.md). The manifest records candidate evidence locations and does not prove execution or deployment adoption.
