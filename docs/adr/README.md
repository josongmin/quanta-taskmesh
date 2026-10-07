# Architecture decisions

| ADR | Status | Authority |
|---|---|---|
| [0001](0001-hexagonal-feature-sliced-architecture.md) | Accepted | Crate and hexagonal boundaries |
| [0002](0002-drr-proportional-fairness.md) | Accepted | Deterministic proportional DRR |
| [0003](0003-sep-16-hardening-contracts.md) | Accepted | D01–D17 hardening contracts including implemented Sep-21 amendments |
| [0004](0004-sep-25-ingress-plan-identity-and-wire.md) | Accepted, library scope | Trust, plan, identity, and wire authorities |
| [0005](0005-sep-25-execution-response-and-custody.md) | Accepted, library scope | Executor, deadline, response, and custody authorities |
| [0006](0006-source-bound-verification-authority.md) | Accepted, current local policy | Static mapping and exact-source proof boundaries |
| [0008](0008-audit-implementation-record.md) | Accepted, historical scope | TM16/SEP21/BG25 implementation and original proof dispositions; absorbs 0007 |
| [0009](0009-runtime-and-evidence-hardening.md) | Accepted, repository scope | Subsequent runtime, queue, producer and compatibility repairs |
| [9000](9000-benchmark-strategy.md) | Proposed qualification | Implemented measurement rules and unresolved performance qualification |

Public API and wire details live in the
[library spec](../taskmesh-library-spec.md) and
[external interface](../taskmesh-external-interface.md). The
[release checklist](../release-checklist.md) owns operator procedure. Completed
ticket narratives and obsolete audits/plans are recoverable by source commit and
byte digest in [document history](../evidence/document-history.json). The
[remaining-work list](../remaining-work.md) owns open work; the
[documentation index](../README.md) supplies navigation without another status copy.
