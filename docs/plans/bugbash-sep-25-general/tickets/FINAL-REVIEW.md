# Plan review and implementation status

## Verdict

Repository implementation is complete. The overall packet remains partial until exact-source CI proof, D1–D9 human review, and external ingress/wire adoption are supplied by their respective authorities.

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
- Current receipt and nightly limitations are stated without upgrading historical proof.

## Current qualification boundary

- The library strict ingress exists; deployment owners must explicitly call it at their untrusted byte boundary.
- Parent-plan stage membership and detached structured concurrency remain outside the engine contract by D4 and the root-child lifetime fixtures.
- Nightly modelcheck, TSan, fuzz, coverage, IAI, and mutation are NOT_RUN pending explicit authorization.

## External adoption boundary

The repository library implementation is complete. Deployment owners still choose their bytes ingress call site, validate parent-plan membership before submission, and negotiate downstream wire versions. The manifest records candidate evidence locations and does not prove semantic sufficiency, execution, or deployment adoption.
