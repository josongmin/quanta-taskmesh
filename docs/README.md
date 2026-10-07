# Taskmesh documentation

| Question | Authority |
|---|---|
| Current unfinished work | [Remaining work](remaining-work.md) |
| Decisions and completed implementation | [ADR index](adr/README.md) |
| Public API / wire / stages | [Library spec](taskmesh-library-spec.md), [external interface](taskmesh-external-interface.md) |
| Runtime/pool inventory | [Runtime inventory](runtime-inventory-baseline.md) and current owner source |
| Commands and required checks | `Justfile`, `tools/gates/inventory.json`, `tools/gates/required.json` |
| CI operations / release procedure | [CircleCI](ci-circleci.md), [release checklist](release-checklist.md) |
| Measurement protocol / acquisition | [ADR 9000](adr/9000-benchmark-strategy.md), [host series](benchmarks/host-series.md) |
| Executable static ownership | [Sep-21](evidence/sep21/README.md), [Sep-25](evidence/sep25/README.md) |
| Retired narratives, receipts and compressed originals | [Git source and original digests](evidence/document-history.json) |

Current behavior is owned by source and contract documents. ADRs retain rationale
and implementation disposition. Historical status and static mapping do not
certify current source. This index does not duplicate a source/CI status snapshot.

In the history catalog, an entry's `source_head` overrides the top-level default.
`disposition` is `deleted` unless stated as `compressed`; `successor` is the
current authority, not an equivalent qualification receipt. Recover original
bytes with `git show <source_head>:<path>` and check the entry's SHA-256. Historical
Sep-16 receipts and Sep-22 measurements are retained this way, outside the working
tree. The active maps and validators remain executable under `docs/evidence`.
