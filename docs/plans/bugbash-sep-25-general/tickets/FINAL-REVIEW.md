# Plan review and implementation status

## Verdict

The 104-row static mapping validates on the current tree; this is structural evidence, not an execution receipt. New compound fixtures provide candidate evidence for mixed-substrate overload, independent capacity saturation, staged blocker release, policy interactions, concurrent histories, split child lifetimes, and host/simulator accounting. The in-repository semantic audit now has a bounded combined H04 race fixture and a focused Governor schedule persistence/replay fixture for H16. The full nightly modelcheck profile has not run. The macOS CI profile passed at committed, clean HEAD `2555aca6711575449ffe5496e2665bb341ea7c45`; its receipt is valid only for that source and must be reissued after any tracked edit. D1–D9 human review and external ingress/planner/wire adoption remain OPEN under their external authorities. The repaired Semantica concurrent consumer test passes 2/2 on clean Semantica `0384053` against clean Taskmesh `2555aca`; its QBC invocation has manual comparison authority rather than owner/deployment qualification.

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

## Remaining work, in order

| Owner | Required result | Stop condition |
|---|---|---|
| BG25-009/012 | Run and validate the complete source-bound nightly modelcheck producer when high-cost qualification is authorized. | Focused Governor replay does not prove that every registered Loom/Shuttle model completed. |
| External consumer | Obtain owner/deployment qualification for the repaired Semantica path and resolve D1–D9 against actual deployment boundaries. | Clean QBC 2/2 is focused execution with manual comparison authority; broad lib-test is independently blocked by unrelated `E0432`; see `EXTERNAL-ADOPTION.md`. |
| BG25-001/002/003 | Record human D1–D9 review and the actual deployment ingress, planner, and wire consumer call sites with owner-local execution evidence. | Library-local tests cannot close external adoption. |
| BG25-011 | Record a quiet-host environment and baseline only if a latency or throughput qualification claim is required. | Simulator admission wait is not host latency; a first baseline is not a regression PASS. |
| BG25-012 | After the final tracked edit, reissue and validate the clean exact-HEAD macOS CI-profile receipt; retain durable artifact custody. | The `3ac3ac4` receipt passed, but cannot qualify a later HEAD/tree/path digest. |

## External adoption boundary

The repository library implementation is complete for accepted contracts. Deployment owners still choose their bytes ingress call site, validate parent-plan membership before submission, and negotiate downstream wire versions; see [EXTERNAL-ADOPTION.md](EXTERNAL-ADOPTION.md). The manifest records candidate evidence locations and does not prove execution or deployment adoption.
