# Completed work archive — 2026-09-25

These packets preserve implementation and audit history. Their ticket statuses
do not qualify a later source revision, external consumer, nightly run, or
release. Durable decisions are indexed in [ADR 0004](../../adr/0004-sep-25-ingress-plan-identity-and-wire.md),
[ADR 0005](../../adr/0005-sep-25-execution-response-and-custody.md), and
[ADR 0006](../../adr/0006-source-bound-verification-authority.md).

| Packet | Archived fact | Remaining authority |
|---|---|---|
| [Jun-4 startup](jun-4-startup/README.md) | Ten original build tickets | Current public specs and ADR 0001–0002 |
| [Sep-16 hardening](sep-16-hardening/tickets/README.md) | 21 implemented tickets and one retired ticket | ADR 0003; historical receipt and exception register retain their recorded limits |
| [Sep-22 test optimization](sep-22-test-optimization/tickets/README.md) | Eight locally verified implementation tickets | [Qualification plan](../../plans/2026-09-24-ci-verification-stages.md) and exact-source receipt |
| [S25 engine coverage](sep-25-engine-coverage/tickets/README.md) | Superseded draft compressed to a history marker; original 12 tickets remain in Git history | [BG25 final packet](../../plans/bugbash-sep-25-general/tickets/README.md) |
| BG25 implementation | Twelve implemented library tickets compressed into [ADR 0007](../../adr/0007-sep-25-implementation-closure.md); original narratives remain in Git `cbf9764` | [Live manifest](../../plans/bugbash-sep-25-general/tickets/scenario-evidence.json), [external adoption](../../plans/bugbash-sep-25-general/tickets/EXTERNAL-ADOPTION.md), and a new exact-HEAD CI receipt |

The 40 Sep-16 source finding records stay at
[`docs/bugbash/sep-16-general/tickets/`](../../bugbash/sep-16-general/tickets/README.md).
The Sep-16 manifest binds each finding's original path and SHA-256, so those
immutable audit inputs remain an **in-place evidence archive**. SEP-21 tickets
remain active at [`docs/bugbash/sep-21/tickets/`](../../bugbash/sep-21/tickets/README.md):
release qualification is NO-GO and the release tooling reads those paths.

Archive relocation does not change any original source HEAD or historical
receipt authority. A later gate receipt must identify the post-relocation HEAD.
