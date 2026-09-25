# Plan review and implementation status

## Verdict

The packet remains implementable without treating all 53 incomplete scenarios as defects. Implementation has started, and production changes remain limited to reproduced contract failures.

## Classification

| Class | Tickets | Source-change rule |
|---|---|---|
| Contract decision | BG25-001, 003, 007 | Documentation/fixtures first; production behavior changes only after compatibility decision. |
| Resolved structural defect | BG25-005 H30/B25/H27 | `bf03efc` and `3e57f1a` freeze executor authority, preflight Tokio-backed dispatch, and prove aggregate shared-executor bounds. |
| Resolved deadline defect | BG25-006 H34 | `74a18f7` enforces `CompleteBy` at caller response while retaining worker custody through teardown. |
| Conditional contract defect | BG25-002 | Implement only after the official untrusted-byte ingress boundary is identified. |
| Authority migration | BG25-004 B28 | Reproduce collision, choose semver path, then introduce owner-bound handles. |
| Proof-first | BG25-008, 009, 010 | Do not edit production source unless the independent oracle finds a mismatch. |
| Measurement/evidence | BG25-011, 012 | Do not alter runtime semantics to make metrics or gates pass. |

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

## Current open boundary

- BG25-004 raw identity handles still require an API/semver choice before source changes.
- BG25-002 strict ingress still requires the actual external bytes owner and compatibility boundary.
- BG25-006 remains partial until its independent combined event-ledger oracle covers D05, D15, H11, H12, H19, H20, and H31 together.
- BG25-008/009/010 and BG25-011 remain proof or measurement work. They are not confirmed source defects without a failing independent oracle.

## Remaining external dependencies

The plan cannot close deployment-level strict ingress, parent-plan membership, or downstream wire compatibility until the actual external owner and source are identified. Those rows remain OPEN; library-local fixtures must not be promoted to adoption evidence.
