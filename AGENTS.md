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
