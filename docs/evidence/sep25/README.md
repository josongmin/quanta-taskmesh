# Sep-25 static scenario evidence

`ticket-map.json` records the historical 12-ticket implementation ownership;
`scenario-evidence.json` maps 104 scenarios to candidate cases and gate selectors.
These are executable **static provenance**, not current implementation plans or
execution receipts. `MAPPED` does not mean PASS.

Run `validate_ticket_map.py` and `validate_scenario_evidence.py` through
`just prompt-check`. The source-bound proof rules are in
[ADR 0006](../../adr/0006-source-bound-verification-authority.md), completed
implementation in [ADR 0007](../../adr/0007-sep-25-implementation-closure.md),
and remaining work in the
[current plan](../../plans/2026-10-03-current-source-remediation.md).

The old narrative tickets and coverage/command tables are removed from the
working tree; Git history at `cbf9764f08eb9f307f3ee5a51bad5e5dabeaf4a6`
preserves their text. The original 104-row oracle inventory remains at
[`docs/misc/tmp-engine-checklist-sep-25.md`](../../misc/tmp-engine-checklist-sep-25.md)
as historical audit input.
