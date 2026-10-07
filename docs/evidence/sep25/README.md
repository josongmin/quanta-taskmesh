# Sep-25 static scenario evidence

[ticket-map.json](ticket-map.json) records 12 implemented BG25 ownership rows;
[scenario-evidence.json](scenario-evidence.json) retains 104 target/case/oracle/gate
mappings. [scenarios.md](scenarios.md) defines all 104 scenarios and the historical
K 51 / P 34 / G 19 selection baseline. MAPPED means static provenance, not PASS.

`just test-architecture` runs both validators. Source-bound proof is owned by
[ADR 0006](../../adr/0006-source-bound-verification-authority.md), implementation
by [ADR 0007](../../adr/0007-sep-25-implementation-closure.md), and unfinished
work by [remaining work](../../remaining-work.md). The relocated scenario path
changes static input identities; historical receipts remain unmodified.

Deleted narratives and superseded audit tables are recoverable from their Git
source and original digests in [document history](../document-history.json).
