# Prompt context policy

This directory registers, builds, and validates the repository's agent instruction routing.
Detailed contracts remain in their owning ADR/spec/tool files; PM never copies their content
into startup context.

## Structure

- `AGENTS.md` contains only cross-tool project invariants and a short context router.
- `.claude/CLAUDE.md` imports `AGENTS.md`; it does not copy the project rules. Keeping the
  wrapper under `.claude/` avoids making it another root instruction surface for other tools.
- `.cursor/rules/*.mdc` adds only the two areas that benefit from Cursor's path-scoped rules.
- `inventory.json` (schema 2) owns startup budgets/imports, Cursor scopes, and generated-output
  budgets. `generated` lists only surfaces whose marked router blocks PM may update.
- `references.json` (schema 1) registers stable document IDs, repository paths, contract roles,
  and lifecycle (`active`, `superseded`, `archive`). A superseded entry needs `superseded_by`;
  successor cycles are rejected. Entries without a successor use `null`.
- `routes.json` (schema 1) maps unique task conditions to document IDs, optional exact Markdown
  heading text, and instruction surfaces. Every route appears in `AGENTS.md`; Cursor surfaces
  receive only their assigned routes. Conditions guide selection; globs do not infer task intent.

## Build and check

```sh
just prompt-build
just prompt-check
uv run pytest -q tools/pm/tests
```

`prompt-build` validates registries, target files, sections, surfaces, marker pairs, and all
candidate budgets before writing. It replaces only content between `<!-- pm:routes:start -->`
and `<!-- pm:routes:end -->`; manual rules, frontmatter and contract documents remain owned by
their authors. Repeated builds with the same inputs update zero files. Validation failure before
writing leaves outputs intact; this is not a multi-file filesystem transaction.

`prompt-check` is read-only: it renders expected routers in memory and rejects drift. It also
rejects missing/ambiguous sections (headings in fenced examples do not count), inactive route
targets, active references into `archive/`, duplicate IDs/paths/conditions/targets, unknown
fields, and symlinked registries or reference files. The existing `py-test` surface runs the PM
tests, including live drift checks; build is explicit and is not a gate or CI dependency.

## Reference maintenance

1. Add a document only when a registered task route needs it. Give it a stable ID and an exact
   repository path; keep the detailed contract in that document.
2. Add or change the route condition and targets. `section: null` means use relevant parts of
   the document; a string selects an exact unique heading, not a line number or URL fragment.
3. For a move, update the reference path. For replacement, register the successor, mark the old
   entry `superseded`, and switch active routes to the successor. Do not route to archived inputs.
4. Run build, check and focused PM tests; review registry changes and the generated diff together.

Normal references follow current documents and do not pin body digests. Editing document prose
needs no digest refresh; renaming/removing a selected heading requires a route update. Historical
campaign snapshots retain their own commit/digest authority in their plan validator. They are
not enrolled or rewritten by this current-document router. The registry covers routed references,
not all `docs/`, links inside referenced documents, or arbitrary paths in manually written rules.

Run `just prompt-check` after changing these surfaces. The check rejects unregistered Cursor
rules, duplicate or unsupported frontmatter, eager startup imports outside the inventory,
startup budget overflow, symlinked inventory, prompt files or imports, and scope drift. It is
read-only.

Root `AGENTS.override.md`, `CLAUDE.md`, and legacy `.cursorrules` must also be registered if
present. Imported startup files must be inventoried, with no import cycles. Budgets must be
positive integers. This check covers the listed repository startup surfaces and Cursor rules;
it does not inspect personal instructions or prove what an installed agent actually loaded.

The on-demand SEP-21 packet pack is checked separately by
`uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only`: assigned acceptance
coverage, producer signals, local links, and direct shell-block `just` recipe names. It does
not enforce write ownership or qualify source, runtime behavior, or agent instruction following.

The design follows the official guidance current on 2026-09-23:

- OpenAI: keep `AGENTS.md` current, use progressive disclosure, and read only what the task
  needs: <https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra>
- Cursor: use scoped `.cursor/rules/*.mdc` files and references instead of copied context:
  <https://cursor.com/docs/rules>
- Claude Code: keep startup memory concise, import shared instructions, and use path-scoped
  `.claude/rules/` for conditional context: <https://code.claude.com/docs/en/memory>

Prompt files guide agent behavior; they are not an enforcement boundary for runtime, security,
or release policy. Those remain owned by code, hooks, and verification gates.
