# 0004. Untrusted ingress, plan ownership, identity, and wire evolution

- Status: Accepted for the Taskmesh library; external adoption remains open
- Date: 2026-09-25
- Amends: [0001](0001-hexagonal-feature-sliced-architecture.md) at the host ingress boundary
- Source: [BG25 implementation map](0007-sep-25-implementation-closure.md), D1–D5 and D9; original narratives remain at Git `cbf9764`

## Context

Raw public DTOs retain Serde compatibility. That compatibility does not make raw
deserialization a safe authority boundary for untrusted task or runtime JSON.
Taskmesh also cannot infer a parent's declared plan from a child submission, and
numeric permit and ticket sequences from different Governors can collide.
Snapshot schema versioning does not version every other public enum.

## Decision

1. The official untrusted bytes boundary is the opt-in host adapter
   `parse_task_spec` or `parse_runtime_config`. It rejects oversized input before
   parsing, bounds nesting and stage count, rejects duplicate and unknown keys,
   and validates the typed result. Raw DTO Serde remains a compatibility surface,
   not an authorization path. The contract and engine crates do not acquire a
   JSON parser dependency.
2. Strict child input explicitly supplies `parent_awaits`, including `false`,
   plus the exact parent identity. Rust builders retain `child_of` and
   `awaited_child_of` as distinct calls. Strict blocking input explicitly
   chooses `shared_blocking` or `requested_stack` with a stack size. Raw
   `TaskSpec::stack_size_bytes` remains a separate compatibility surface.
3. The external planner owns membership of a child stage in the parent's
   declared plan. The engine owns live parent generation, exact ancestry,
   recursion, and declared wait-cycle checks. Engine membership authority would
   require a separately designed retained parent-plan registry.
4. Permit and ticket authority is Governor-bound and opaque. A display or
   telemetry sequence is not an authorization handle. Foreign handles reject
   even when local sequence numbers match; sequence exhaustion fails closed.
5. `Snapshot.schema_version` versions Snapshot only. Consumers must handle
   decode or version failure for Snapshot and independently evolving verdict or
   terminal enums. A tolerant envelope requires a separate versioned contract.

## Consequences and limits

- Deployment owners must identify each actual bytes ingress, planner, and wire
  consumer. A library-local test does not prove that a deployed consumer uses
  the strict adapter or handles version failure.
- Strict ingress is additive. Tightening raw DTO Serde silently would change a
  different contract and needs compatibility review.
- The library's choices above are accepted; migration approval and deployed
  consumer qualification are recorded in the
  [external adoption ledger](../plans/bugbash-sep-25-general/tickets/EXTERNAL-ADOPTION.md).
- The exact public fields, validation limits, and error variants belong to the
  [library spec](../taskmesh-library-spec.md) and
  [external interface](../taskmesh-external-interface.md). This ADR records why
  these authorities are separated.

## Evidence boundary

The [BG25 implementation map](0007-sep-25-implementation-closure.md) and
[104-scenario inventory](../misc/tmp-engine-checklist-sep-25.md)
retain the compressed implementation history and candidate fixtures. Their static mapping
does not certify an external deployment or a later source revision.
