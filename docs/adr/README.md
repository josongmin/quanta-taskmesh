# Architecture decisions

| ADR | Status | Authority |
|---|---|---|
| [0001](0001-hexagonal-feature-sliced-architecture.md) | Accepted | Crate and hexagonal boundaries |
| [0002](0002-drr-proportional-fairness.md) | Accepted | Deterministic proportional DRR |
| [0003](0003-sep-16-hardening-contracts.md) | Accepted | Sep-16 contracts; its Sep-21 candidate supplement is not release approval |
| [0004](0004-sep-25-ingress-plan-identity-and-wire.md) | Accepted, library scope | Trust, plan, identity, and wire authorities |
| [0005](0005-sep-25-execution-response-and-custody.md) | Accepted, library scope | Executor, deadline, response, and custody authorities |
| [0006](0006-source-bound-verification-authority.md) | Accepted, current local policy | Static mapping and exact-source proof boundaries |
| [9000](9000-benchmark-strategy.md) | Proposed | Benchmark north star, not current qualification |

Public API and wire details live in the
[library spec](../taskmesh-library-spec.md) and
[external interface](../taskmesh-external-interface.md). The
[release checklist](../release-checklist.md) owns operator procedure. Completed
ticket narratives are in the [archive](../archive/2026-09-25/README.md).
