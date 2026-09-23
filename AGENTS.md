Be concise, direct, and technical.

Prioritize:
- fail-closed behavior
- deterministic outputs
- capability-pool topology
- class-policy clarity
- inventory-backed runtime changes

Rules:

1. Semantic policy and worker governance are separate concerns.
2. Public APIs must stay product-neutral.
3. Unknown classes reject, not default-admit.
4. Competing execution paths do not get unbounded queues.
5. Engine-specific pool proliferation is forbidden.
6. Parallel stages need explicit deterministic reduce policy.
7. Inventory changes must be explicit and reviewable.

Context routing:

- For crate or dependency boundaries, read `docs/adr/0001-hexagonal-feature-sliced-architecture.md`.
- For public API changes, read the relevant part of `docs/taskmesh-library-spec.md`; for wire or
  external behavior, use `docs/taskmesh-external-interface.md`.
- For gate wiring, use `tools/gates/inventory.json` and `tools/gates/required.json`; for a
  release decision, use the relevant section of `docs/release-checklist.md`.
- For ticket work, read the named ticket and its current owner source. Do not preload the
  remaining plan, bugbash, or documentation stack.
