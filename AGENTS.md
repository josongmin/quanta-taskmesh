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
8. Contract owns types/ports; engine owns governance state; host owns execution/custody;
   adapters own executor implementations. Host `run_*` executes only the first declared stage.
9. Caller response completion is not worker termination or capacity release.

Working-source and evidence policy:

- Preserve existing dirty hunks; inspect the owning paths before editing a shared checkout.
- Report verification against its actual source identity and scope. Focused, dirty-source,
  cached, or historical results do not establish final clean-source qualification.
- Keep implementation, verification, release qualification, and deployment/activation states
  distinct; record required checks that failed or were not run.

Verification cost policy:

- Do not run mutation campaigns during ordinary audits, owner-local checks, or daily verification.
- Run `mutants-critical`, `mutants-generated`, or commands that include them (`nightly`,
  `verify-macos-nightly`, `release`, `qualify-local`,
  `tools/gates/run.py --required`, `--all`, `--tier nightly`, or `--profile release`)
  only when the current user request
  explicitly authorizes mutation testing or final release qualification that includes it.
- Use focused owner tests, `dev`, and `verify-macos-ci` for normal feedback. A partial mutation run is
  never qualification evidence.

Context routing (read only the relevant sections):

<!-- pm:routes:start -->
- Crate or dependency boundary changes:
  `docs/adr/0001-hexagonal-feature-sliced-architecture.md`.
- Public API changes: `docs/taskmesh-library-spec.md`.
- Stages, wire or external behavior changes: `docs/taskmesh-external-interface.md`.
- Untrusted ingress or plan identity changes:
  `docs/adr/0004-sep-25-ingress-plan-identity-and-wire.md` (section: Decision).
- Executor, cancellation, deadline or drain changes:
  `docs/adr/0005-sep-25-execution-response-and-custody.md` (section: Decision).
- Gate wiring changes: `tools/gates/inventory.json`; `tools/gates/required.json`.
- Evidence producer or receipt changes: `docs/adr/0006-source-bound-verification-authority.md`
  (section: Decision).
- Release decisions: `docs/release-checklist.md`.
- Benchmark or measurement changes: `docs/adr/9000-benchmark-strategy.md`.
- Selecting checks for changed owner paths: `Justfile`.
<!-- pm:routes:end -->

- For ticket work, read the named ticket and current owner source; do not preload historical plans.
