# Sep-21 historical ownership and release input

The schema-2 [ticket map](ticket-map.json) preserves 12 tickets, 23 findings and
all original acceptance IDs. [ADR 0008](../../adr/0008-audit-implementation-record.md)
replaces ticket narratives and execution packets. Its per-ticket historical
status does not qualify current source. V01/V02/R01 remain unqualified.

`validate_ticket_map.py` checks map/ADR consistency without executing a campaign.
Finding and release producers bind the tracked map path/digest. Moving from the
old `plan.json` changes the input identity: regenerate finding/release artifacts
on the final clean source. Old manifests fail the new identity comparison;
historical receipts are retained without rewriting their bytes.

[Document history](../document-history.json) binds the deleted inputs to their
Git commit and original digests. [Remaining work](../../remaining-work.md) owns
current proof, compatibility and consumer work.
