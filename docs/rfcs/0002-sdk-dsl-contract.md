# RFC 0002: SDK DSL and execution contract — superseded

- Status: **Superseded on 2026-09-27**
- Current proposal: [SEP-27 SDK DX and extensibility](../rfc/sep-27-sdk-dx-extensibility.md)
- Historical text: [SEP-24 proposal archive](../archive/2026-09-27/0002-sdk-dsl-contract.md)

The old AS-IS diagram described source baseline
`e5aa4b63a1fedb3b416d33d1ea7ff364258002b5`. It must not be used as the current
implementation inventory. Duplicate class policy rejection, class validation at
construction, and the direct/host settlement notification are implemented in the
SEP-27 reviewed source. Strict JSON ingress also has byte/depth/stage limits;
those limits do not imply a stage-count bound on every in-process plan path.

The successor records remaining SDK work, compatibility decisions, owner paths,
and acceptance criteria. This path remains as a link target for existing references.
Implemented API and execution contracts remain in the
[library spec](../taskmesh-library-spec.md),
[external interface](../taskmesh-external-interface.md), and
[ADR 0005](../adr/0005-sep-25-execution-response-and-custody.md).
