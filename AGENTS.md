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

Verification cost policy:

- Do not run mutation campaigns during ordinary audits, owner-local checks, or daily verification.
- Run `mutants-critical`, `mutants-generated`, or commands that include them (`nightly`,
  `verify-macos-nightly`, `release`, `qualify-local`,
  `tools/gates/run.py --required`, `--all`, `--tier nightly`, or `--profile release`)
  only when the current user request
  explicitly authorizes mutation testing or final release qualification that includes it.
- Use focused owner tests, `dev`, and `verify-macos-ci` for normal feedback. A partial mutation run is
  never qualification evidence.

Context routing:

- For crate or dependency boundaries, read `docs/adr/0001-hexagonal-feature-sliced-architecture.md`.
- For public API changes, read the relevant part of `docs/taskmesh-library-spec.md`; for wire or
  external behavior, use `docs/taskmesh-external-interface.md`.
- For gate wiring, use `tools/gates/inventory.json` and `tools/gates/required.json`; for a
  release decision, use the relevant section of `docs/release-checklist.md`.
- For ticket work, read the named ticket and its current owner source. Do not preload the
  remaining plan, bugbash, or documentation stack.
