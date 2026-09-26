# Taskmesh benchmark qualification — active plan

- Status: **OPEN / performance UNQUALIFIED**.
- Measurement contract, implemented harness and completed B07 owner-local
  recovery work: [ADR 9000](../../../adr/9000-benchmark-strategy.md).
- Detailed remaining owner actions and completion evidence:
  [B04 remaining audit](B04-remaining-audit.md).
- Original implementation/audit narrative: Git
  `cbf9764f08eb9f307f3ee5a51bad5e5dabeaf4a6`, this path and
  `B07-minimal-recovery-soak.md`.

## Dependency order

1. **B00 / H7:** obtain real consumer SLOs, precision budget, fixed rate grid,
   exclusions, claim scope and representative workload provenance.
2. **B04:** freeze source/build, acquire independent optimized oracle proof,
   calibrate a quiet host, then collect balanced repeated controls, workload
   runs and all rejected attempts. Keep open-loop and closed-loop populations
   distinct. `host_perf.py --require-performance` stays closed until measured
   admission is implemented and satisfied.
3. **B06, conditional on an industry claim:** name a maintained peer, prove
   the semantic intersection, compare on the same declared host and workload,
   and obtain an independent rerun.
4. **B05 and extra modes, conditional on an explicit claim or attribution
   failure:** add bounded phase observation or repeated mode-specific admission
   only with a measured need and frozen budgets.

Current-source clean CI, nightly/release and external deployment have separate
authorities in [ADR 0006](../../../adr/0006-source-bound-verification-authority.md)
and the [release checklist](../../../release-checklist.md). B07's 600-second H2
and post-commit H5 observations are functional owner-local diagnostics, not a
performance result or a longitudinal leak guarantee.
