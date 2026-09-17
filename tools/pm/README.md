# tools/pm — taskmesh prompt-manager

A small validate → plan → apply renderer for generated agent/operator docs.
When a target is declared, its content lives in `sources/**` (currently absent
— see below), is stitched by `targets.yaml`, and rendered through Jinja2
templates under `templates/` into files at the repo root.

## What PM owns here

**Nothing, at present — and that is stated, not hidden.**

- `AGENTS.md` is hand-authored by the repository owner. It is not a PM target
  and `pm sync` will never touch it.
- The sources this tool used to carry described an unrelated project. They were
  removed rather than rendered, because generating `CLAUDE.md`, `.codex/*`, or
  `.cursorrules` from them would have installed wrong instructions for every
  agent in this repository. Opting a generated file in is an owner decision:
  add a target to `targets.yaml`, author its sources, run `pm sync`.

`pm lint` therefore reports `0 target(s) up to date`. That is a true statement
about an empty set, not a green light over unchecked files; the tool itself is
verified against hermetic fixtures in `tests/`.

## Commands

```bash
uv run python tools/pm/pm.py lint      # read-only: drift/missing → exit 1
uv run python tools/pm/pm.py status    # read-only
uv run python tools/pm/pm.py preview --target <name>   # read-only
uv run python tools/pm/pm.py sync      # apply; refuses hand-edited outputs
uv run python tools/pm/pm.py sync --force   # replace hand-edited outputs
```

## Contract

- **Manifest validation happens at parse time.** Duplicate YAML keys are
  refused while the mapping is being built (after `safe_load` the first
  declaration is already gone). Unknown keys, absolute paths, `..` components,
  templates outside `templates/`, and two targets writing one output are errors.
- **Template identity is the path.** `templates/nested/x.j2` and
  `templates/x.j2` are different templates; the one the manifest names is the
  one rendered and the one lint checks. The resolved path must stay inside the
  template root (symlinks are resolved before the check).
- **One plan for lint and sync.** Both build the same `RenderPlan` (exact
  template path, section digests, rendered digest, output path). Lint compares
  it to disk and writes nothing; sync applies it.
- **Sync is safe by default.** Every target is planned before any file is
  written. A missing output is created; a matching one is left alone; a
  differing one is *refused* unless `--force`. Writes are atomic
  (temp file + rename) and reported per file, so a partial apply is never silent.

## Layout

```text
tools/pm/
  pm.py
  targets.yaml       # currently: targets: {}
  templates/         # _banner.j2 (shared banner include)
  tests/             # hermetic fixtures + the audit's retained reproductions
```
