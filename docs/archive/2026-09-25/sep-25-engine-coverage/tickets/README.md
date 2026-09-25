# S25 superseded draft — history marker

The S25 packet was a documentation-only proposal at
`76559c483bdd1d6b0b5226f7c5b5591b919afae9`. Its 12 tickets assigned the
53 P/G scenarios from the [104-case checklist](../../../../misc/tmp-engine-checklist-sep-25.md).
It produced no implementation or qualification evidence. The
[BG25 execution packet](../../../../plans/bugbash-sep-25-general/tickets/README.md)
replaced its ticket ownership and scenario mapping; [ADR 0004](../../../../adr/0004-sep-25-ingress-plan-identity-and-wire.md),
[ADR 0005](../../../../adr/0005-sep-25-execution-response-and-custody.md), and
[ADR 0006](../../../../adr/0006-source-bound-verification-authority.md)
record the accepted boundaries.

The supersession pointer remains in [plan.json](plan.json) because the active
BG25 validator checks that only one execution plan has authority. The removed
draft text is recoverable from Git commit `1704b261d9526a59f85c52fbaf575b9cfff7a355`
under this directory. Do not execute or aggregate S25 as a separate plan.
