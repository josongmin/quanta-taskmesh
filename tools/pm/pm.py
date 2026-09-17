#!/usr/bin/env python3
"""taskmesh prompt-manager.

Renders generated agent/operator docs from ``tools/pm/sources/**`` through
jinja2 templates into target files declared in ``targets.yaml``.

Commands:

- ``lint``     re-render every target in memory and fail closed if any output
               drifted or is missing. **Read-only**: it never touches a target.
- ``status``   list targets, output paths, and ok/drift state (read-only).
- ``preview``  print the rendered output for one target (read-only).
- ``sync``     apply the render plan: write every target. Refuses to overwrite
               a file that changed since the plan was computed, refuses to
               overwrite a hand-edited file unless ``--force``, writes each file
               atomically, and reports per-file results so a partial apply is
               never silent.

# Validate → plan → apply

Every command builds the same ``RenderPlan`` first (``plan_all``). The plan
carries the *exact* template path (relative to the template root, never the
basename), the section source digests, the rendered content and its digest, and
the output path. ``lint`` compares the plan to disk; ``sync`` applies it. There
is no way for lint to validate one thing and sync to write another.

# Configuration is validated at parse time

``targets.yaml`` is loaded with a strict YAML loader that rejects a duplicate
mapping key *while the mapping is being built* — after ``yaml.safe_load`` has
collapsed the dict, the first declaration is already gone and no later check can
recover it. Unknown keys, non-relative or escaping paths, and two targets that
normalize to the same output are configuration errors, not something to notice
in a diff later.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

import yaml
from jinja2 import (
    Environment,
    FileSystemLoader,
    StrictUndefined,
    TemplateNotFound,
    TemplateSyntaxError,
    select_autoescape,
)

# tools/pm/pm.py -> PM_DIR = tools/pm, REPO_ROOT = repo root (parent of tools/).
PM_DIR = Path(__file__).resolve().parent
REPO_ROOT = PM_DIR.parent.parent

TARGET_KEYS = {"output", "template", "sections"}
TOP_LEVEL_KEYS = {"targets"}


class PmError(Exception):
    """Raised for configuration/usage errors (missing files, unknown target)."""


# ---- strict YAML -----------------------------------------------------------


class _StrictLoader(yaml.SafeLoader):
    """A SafeLoader that refuses duplicate mapping keys.

    ``yaml.safe_load`` keeps the last value for a repeated key. For a targets
    manifest that means a copy-pasted target silently deletes the one above it
    from the lint set. The check has to happen here, during construction, because
    afterwards there is nothing left to compare.
    """

    def construct_mapping(self, node, deep=False):  # type: ignore[override]
        if not isinstance(node, yaml.MappingNode):
            raise PmError(
                f"expected a mapping at line {node.start_mark.line + 1}, "
                f"got {node.__class__.__name__}"
            )
        seen: dict[object, int] = {}
        for key_node, _value_node in node.value:
            key = self.construct_object(key_node, deep=deep)
            if key in seen:
                raise PmError(
                    f"duplicate mapping key {key!r} at line {key_node.start_mark.line + 1} "
                    f"(first declared at line {seen[key]})"
                )
            seen[key] = key_node.start_mark.line + 1
        return super().construct_mapping(node, deep=deep)


def strict_yaml_load(text: str):
    try:
        return yaml.load(text, Loader=_StrictLoader)  # noqa: S506 - SafeLoader subclass
    except yaml.YAMLError as exc:
        raise PmError(f"invalid YAML: {exc}") from exc


# ---- targets ---------------------------------------------------------------


@dataclass(frozen=True)
class Target:
    name: str
    output: str
    template: str
    sections: tuple[str, ...]


def _validated_relative(kind: str, name: str, raw: str) -> str:
    """A path that stays inside the root it is resolved against.

    Returned normalized (``a/./b`` and ``a/b`` are the same target). Rejects
    absolute paths and any ``..`` component: the containment policy (D11) is
    that outputs live under the repo root and templates/sections under
    ``tools/pm``, and a manifest cannot opt out of that.
    """
    if not isinstance(raw, str) or not raw.strip():
        raise PmError(f"target '{name}': {kind} must be a non-empty string")
    posix = PurePosixPath(raw.replace(os.sep, "/"))
    if posix.is_absolute():
        raise PmError(f"target '{name}': {kind} must be relative, got {raw!r}")
    parts = [p for p in posix.parts if p not in ("", ".")]
    if ".." in parts:
        raise PmError(f"target '{name}': {kind} must not contain '..', got {raw!r}")
    if not parts:
        raise PmError(f"target '{name}': {kind} is empty after normalization ({raw!r})")
    return "/".join(parts)


def load_targets(targets_path: Path) -> dict[str, Target]:
    """Load and validate ``targets.yaml`` into a name -> Target mapping."""
    if not targets_path.is_file():
        raise PmError(f"targets file not found: {targets_path}")
    raw = strict_yaml_load(targets_path.read_text(encoding="utf-8")) or {}
    if not isinstance(raw, dict):
        raise PmError(f"targets file must be a mapping at the top level: {targets_path}")
    unknown_top = set(raw) - TOP_LEVEL_KEYS
    if unknown_top:
        raise PmError(f"targets file has unknown top-level keys: {sorted(unknown_top)}")
    targets_raw = raw.get("targets")
    if targets_raw is None:
        targets_raw = {}
    if not isinstance(targets_raw, dict):
        raise PmError(f"targets file has no 'targets' mapping: {targets_path}")

    targets: dict[str, Target] = {}
    outputs_seen: dict[str, str] = {}
    for name, spec in targets_raw.items():
        if not isinstance(name, str) or not name:
            raise PmError(f"target names must be non-empty strings, got {name!r}")
        if not isinstance(spec, dict):
            raise PmError(f"target '{name}' must be a mapping")
        missing = TARGET_KEYS - set(spec)
        if missing:
            raise PmError(f"target '{name}' missing required key(s) {sorted(missing)}")
        unknown = set(spec) - TARGET_KEYS
        if unknown:
            raise PmError(f"target '{name}' has unknown key(s) {sorted(unknown)}")
        sections = spec["sections"]
        if not isinstance(sections, list) or not sections:
            raise PmError(f"target '{name}' must have a non-empty 'sections' list")
        output = _validated_relative("output", name, spec["output"])
        template = _validated_relative("template", name, spec["template"])
        if not template.startswith("templates/"):
            raise PmError(f"target '{name}': template must live under templates/, got {template!r}")
        normalized_sections = tuple(
            _validated_relative(f"sections[{i}]", name, s) for i, s in enumerate(sections)
        )
        if len(set(normalized_sections)) != len(normalized_sections):
            raise PmError(f"target '{name}' lists the same section more than once")
        if output in outputs_seen:
            raise PmError(f"targets '{outputs_seen[output]}' and '{name}' both write {output!r}")
        outputs_seen[output] = name
        targets[name] = Target(
            name=name, output=output, template=template, sections=normalized_sections
        )
    return targets


# ---- planning --------------------------------------------------------------


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


@dataclass(frozen=True)
class RenderPlan:
    """Everything needed to lint or apply one target, computed once."""

    target: Target
    template_path: Path
    """The exact template file that was rendered (not its basename)."""
    section_digests: tuple[str, ...]
    rendered: str
    rendered_digest: str
    output_path: Path


def _read_sections(pm_dir: Path, target: Target) -> tuple[str, tuple[str, ...]]:
    chunks: list[str] = []
    digests: list[str] = []
    for section in target.sections:
        section_path = pm_dir / section
        if not section_path.is_file():
            raise PmError(f"target '{target.name}': missing source file {section_path}")
        data = section_path.read_bytes()
        digests.append(_digest(data))
        chunks.append(data.decode("utf-8").rstrip("\n"))
    return "\n\n".join(chunks) + "\n", tuple(digests)


def _environment(pm_dir: Path) -> Environment:
    return Environment(
        loader=FileSystemLoader(str(pm_dir / "templates")),
        autoescape=select_autoescape(enabled_extensions=()),
        undefined=StrictUndefined,
        keep_trailing_newline=True,
    )


def plan_target(target: Target, pm_dir: Path = PM_DIR, repo_root: Path = REPO_ROOT) -> RenderPlan:
    """Resolve and render one target without writing anything."""
    template_root = (pm_dir / "templates").resolve()
    template_path = (pm_dir / target.template).resolve()
    # Containment is re-checked on the resolved path: a symlink under
    # templates/ that points outside it is still outside it.
    if template_root not in template_path.parents:
        raise PmError(
            f"target '{target.name}': template {target.template!r} resolves outside {template_root}"
        )
    if not template_path.is_file():
        raise PmError(f"target '{target.name}': missing template {template_path}")
    body, section_digests = _read_sections(pm_dir, target)
    # The loader-relative path preserves nested identity: templates/a/x.j2 and
    # templates/b/x.j2 are different templates, and the one the manifest named
    # is the one rendered — not whichever the loader finds first by basename.
    loader_relative = template_path.relative_to(template_root).as_posix()
    env = _environment(pm_dir)
    try:
        template = env.get_template(loader_relative)
        rendered = template.render(target_name=target.name, output=target.output, body=body)
    except TemplateNotFound as exc:
        raise PmError(
            f"target '{target.name}': template loader could not find {loader_relative!r} "
            f"under {template_root}: {exc}"
        ) from exc
    except TemplateSyntaxError as exc:
        raise PmError(
            f"target '{target.name}': template {template_path} line {exc.lineno}: {exc.message}"
        ) from exc
    if not rendered.endswith("\n"):
        rendered += "\n"
    return RenderPlan(
        target=target,
        template_path=template_path,
        section_digests=section_digests,
        rendered=rendered,
        rendered_digest=_digest(rendered.encode("utf-8")),
        output_path=output_path(target, repo_root),
    )


def plan_all(targets: dict[str, Target], pm_dir: Path, repo_root: Path) -> dict[str, RenderPlan]:
    """Plan every target. Any error aborts before a single byte is written."""
    return {name: plan_target(targets[name], pm_dir, repo_root) for name in sorted(targets)}


def render_target(target: Target, pm_dir: Path = PM_DIR) -> str:
    """Render a single target to a string (does not write to disk)."""
    return plan_target(target, pm_dir, REPO_ROOT).rendered


def output_path(target: Target, repo_root: Path = REPO_ROOT) -> Path:
    resolved_root = repo_root.resolve()
    out = (resolved_root / target.output).resolve()
    if resolved_root != out and resolved_root not in out.parents:
        raise PmError(
            f"target '{target.name}': output {target.output!r} resolves outside {resolved_root}"
        )
    return out


def drift_state(plan: RenderPlan) -> str:
    """'ok', 'drift', or 'missing' for a planned target. Read-only."""
    if not plan.output_path.is_file():
        return "missing"
    current = plan.output_path.read_bytes()
    return "ok" if _digest(current) == plan.rendered_digest else "drift"


def check_drift(target: Target, pm_dir: Path = PM_DIR, repo_root: Path = REPO_ROOT) -> str:
    """Return a drift state for ``target``: 'ok', 'drift', or 'missing'."""
    return drift_state(plan_target(target, pm_dir, repo_root))


# ---- applying --------------------------------------------------------------


@dataclass(frozen=True)
class ApplyResult:
    name: str
    output: Path
    action: str
    """'created', 'updated', 'unchanged', or 'refused'."""
    reason: str = ""


def _atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".pm-tmp")
    tmp.write_bytes(data)
    os.replace(tmp, path)


def apply_plan(plan: RenderPlan, *, force: bool = False) -> ApplyResult:
    """Write one planned target.

    - A missing output is created.
    - An output that already matches is left alone.
    - An output that differs is **refused** unless ``force``: a differing file
      is either a hand edit (which the caller must decide to discard) or a
      concurrent change. Either way, silently replacing it is wrong.
    """
    out = plan.output_path
    data = plan.rendered.encode("utf-8")
    if out.exists():
        if not out.is_file():
            return ApplyResult(
                plan.target.name, out, "refused", "output exists and is not a regular file"
            )
        current = _digest(out.read_bytes())
        if current == plan.rendered_digest:
            return ApplyResult(plan.target.name, out, "unchanged")
        if not force:
            return ApplyResult(
                plan.target.name,
                out,
                "refused",
                "existing file differs from the render (hand edit or concurrent change); "
                "pass --force to replace it",
            )
        _atomic_write(out, data)
        return ApplyResult(plan.target.name, out, "updated")
    _atomic_write(out, data)
    return ApplyResult(plan.target.name, out, "created")


def sync_target(target: Target, pm_dir: Path, repo_root: Path) -> Path:
    """Render ``target`` and write its output file (forced). Returns the path.

    Kept for the hermetic tests; the CLI goes through ``apply_plan`` with the
    refusal semantics above.
    """
    plan = plan_target(target, pm_dir, repo_root)
    _atomic_write(plan.output_path, plan.rendered.encode("utf-8"))
    return plan.output_path


# ---- commands --------------------------------------------------------------


def _load(targets_path: Path) -> dict[str, Target]:
    return load_targets(targets_path)


def cmd_sync(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    # Plan everything first: a broken target must fail before any file changes.
    plans = plan_all(targets, args.pm_dir, args.repo_root)
    results = [apply_plan(plans[name], force=args.force) for name in sorted(plans)]
    refused = [r for r in results if r.action == "refused"]
    for result in results:
        line = f"{result.action:<9} {result.name} -> {result.output}"
        if result.reason:
            line += f"  ({result.reason})"
        print(line, file=sys.stderr if result.action == "refused" else sys.stdout)
    if refused:
        print(
            f"pm sync: {len(refused)} target(s) refused; {len(results) - len(refused)} applied. "
            "Nothing was partially written: each file is replaced atomically.",
            file=sys.stderr,
        )
        return 1
    return 0


def cmd_lint(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    plans = plan_all(targets, args.pm_dir, args.repo_root)
    drifted: list[str] = []
    for name in sorted(plans):
        state = drift_state(plans[name])
        if state != "ok":
            drifted.append(f"{name} ({state}) -> {targets[name].output}")
    if drifted:
        print("pm lint FAILED: generated docs are out of date.", file=sys.stderr)
        for line in drifted:
            print(f"  - {line}", file=sys.stderr)
        print(
            "run `uv run python tools/pm/pm.py sync` (or `python3 tools/pm/pm.py sync`)",
            file=sys.stderr,
        )
        return 1
    print(f"pm lint OK: {len(targets)} target(s) up to date")
    return 0


def cmd_status(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    plans = plan_all(targets, args.pm_dir, args.repo_root)
    print(f"{'TARGET':<14} {'STATE':<8} OUTPUT")
    for name in sorted(plans):
        print(f"{name:<14} {drift_state(plans[name]):<8} {targets[name].output}")
    return 0


def cmd_preview(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    if args.target not in targets:
        raise PmError(
            f"unknown target '{args.target}' (known: {', '.join(sorted(targets)) or 'none'})"
        )
    sys.stdout.write(plan_target(targets[args.target], args.pm_dir, args.repo_root).rendered)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="pm",
        description="taskmesh prompt-manager: render agent/operator docs from sources.",
    )
    parser.add_argument(
        "--targets", type=Path, default=PM_DIR / "targets.yaml", help="path to targets.yaml"
    )
    parser.add_argument(
        "--pm-dir", type=Path, default=PM_DIR, help="directory holding sources/ and templates/"
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="repo root that output paths are resolved against",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sync = sub.add_parser("sync", help="apply the render plan (refuses to overwrite hand edits)")
    sync.add_argument(
        "--force",
        action="store_true",
        help="replace outputs that differ from the render (discarding hand edits)",
    )
    sync.set_defaults(func=cmd_sync)
    sub.add_parser("lint", help="fail closed if any target drifted (read-only)").set_defaults(
        func=cmd_lint
    )
    sub.add_parser("status", help="list targets and drift state (read-only)").set_defaults(
        func=cmd_status
    )
    preview = sub.add_parser("preview", help="print a rendered target to stdout (read-only)")
    preview.add_argument("--target", required=True, help="target name to preview")
    preview.set_defaults(func=cmd_preview)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except PmError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
