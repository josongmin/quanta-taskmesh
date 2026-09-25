# 0005. Execution authority, response deadlines, and worker custody

- Status: Accepted for the Taskmesh library; external adoption remains open
- Date: 2026-09-25
- Amends: [0003](0003-sep-16-hardening-contracts.md), especially D05 and D09–D10
- Source: [BG25 decisions](../archive/2026-09-25/bugbash-sep-25-general/tickets/DECISIONS.md), D6–D8

## Context

An executor's declaration, the capacity charged by the Governor, the worker's
actual execution, the caller's response, and final cleanup are different facts.
Conflating them permits execution under an unvalidated adapter or makes a
response wait for a blocking child that cannot be aborted. A runtime can govern
only the submissions within its capacity authority.

## Decision

1. `Builder` validates and freezes one executor descriptor. Preflight, physical
   requirements, dispatch, and the exposed descriptor consume that same value.
   Default Tokio-backed blocking and CPU paths reject missing Tokio context
   with a typed error before admission or worker creation. Rayon and custom
   adapters declare their own prerequisites.
2. `RunFor` remains a budget measured from worker start. For blocking work it
   bounds the caller's wait; it does not stop an already started synchronous
   worker. `CompleteBy` is an absolute bound through caller response. If owned
   runtime teardown crosses it, the caller receives a terminal deadline result
   while the worker keeps its lease until teardown ends. A late success is not
   returned after `CompleteBy`.
3. Cancellation, deadline, caller drop, root completion, child completion, and
   lease release are distinct transitions. Normal success is visible only after
   the required release fence. A terminal caller response may precede worker
   cleanup; its charged capacity remains observable until cleanup finishes.
4. Semantic capacity and physical worker-domain capacity are reserved in one
   Governor transition. Each runtime bounds its own submissions. A combined
   bound across runtimes exists only when they share an explicit capacity
   authority. Ambient tasks outside that authority are outside the claim.
   Engine-specific pools and a second unbounded host queue are not substitutes.
5. A stage or reduce declaration is governance metadata. One `run_*` call
   executes one submitted unit. The caller owns subsequent submissions,
   fan-out, reducer execution, and detached-child completion unless it explicitly
   awaits or governs them. A parallel declaration still requires a deterministic
   reduce policy.

## Relation to earlier decisions

ADR 0003 describes the Sep-16 contract and its later Sep-21 candidate section.
This ADR makes the Sep-25 `CompleteBy` caller-response fence and frozen
executor/context authority explicit. It does not retroactively label the
Sep-21 release candidate as approved: compatibility adjudication and release
qualification remain separate decisions.

## Consequences and limits

- No hard real-time OS scheduling guarantee is claimed.
- An adapter's declared worker count is not evidence that an externally shared
  pool has a global bound.
- Deployment owners must check deadline, context, and shared-executor assumptions
  at their actual call sites; see the
  [external adoption ledger](../plans/bugbash-sep-25-general/tickets/EXTERNAL-ADOPTION.md).
- Exact API behavior and error variants remain in the
  [library spec](../taskmesh-library-spec.md) and
  [external interface](../taskmesh-external-interface.md).
