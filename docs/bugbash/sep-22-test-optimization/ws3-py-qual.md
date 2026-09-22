# WS3 — Python qualification: actionable findings

Baseline: `7023e945e1c9b3e7e7cca1b26f7af8467df6ab60`.

Current verification:

```text
uv run pytest tools -q --durations=40
544 passed in 136.17s
```

The prior report's Semgrep CA failure, IAI `mktemp` denial, receipt flake, and 408-pass count are not
current and are removed from the queue.

## TO-05 — declared Python floor is false for generated-mutation tooling (P0)

Evidence:

- `pyproject.toml:5` declares `requires-python = ">=3.9"`.
- `pyproject.toml:11` configures Ruff for Python 3.9.
- `tools/verification/run_generated_mutants.py:14` imports stdlib `tomllib` unconditionally.
- `tomllib` entered the standard library in Python 3.11, so importing the producer and its tests
  fails during collection on a supported 3.9/3.10 interpreter.

Required fix:

- Preferred if 3.9 remains supported: add an explicit conditional `tomli` dependency and import it
  as the fallback.
- Alternative: raise `requires-python` and Ruff's target to 3.11 in the same change.
- Verify with the lowest declared interpreter. A skip in
  `test_run_generated_mutants.py` is not acceptable because it hides a broken shipped tool.

## TO-06 — identical real Semgrep scan executes twice (P1)

`tools/semgrep/tests/test_rules_fire.py:459-488` calls the exact command below independently from two
tests:

```text
semgrep --config tools/semgrep/rules --json --verbose crates
```

Current full-run attribution:

- `test_real_integration_tests_are_inside_the_scanned_target_set`: 8.99 s.
- `test_the_scan_reaches_every_crate_test_directory`: 11.29 s.

Required fix:

- Add one module-scoped fixture returning the real scan's `paths.scanned` set.
- Keep both tests and both diagnostics; only share producer execution.
- Do not reuse the trigger/clean fixture scans: those are different trees and different oracles.

## Explicit non-findings

- Do not mark subprocess/tool tests `slow` and exclude them from `just py-test` without adding a
  separate required inventory gate. The current marker configuration does not filter anything;
  filtering it alone would weaken qualification.
- IAI shell tests are individually distinct fail-closed cases. Their cost is real but no redundant
  producer execution was established.
- The receipt malformed-shape tests exercise different envelope, summary, record, and top-level
  paths. The broad mutation matrix lacks their reason-specific diagnostics; merging bodies is not a
  runtime optimization.
- The `just gate`, `matrix`, and `proof` drift tests cover different dependency expansions and
  failure messages. Parametrization saves lines, not execution.
- `test_render_plan.py` subprocess cost is sub-second in the measured run; replacing the shipped CLI
  surface with in-process calls is not justified.
- The Semgrep module fixture already collapses the fire/clean corpus to two scans. A tool outage is
  one root failure rendered against parametrized dependents; changing report shape is not a test
  optimization.
