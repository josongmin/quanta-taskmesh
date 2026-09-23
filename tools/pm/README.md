# Prompt context policy

This directory validates the repository's agent instruction surfaces. It does not render or
rewrite them.

## Structure

- `AGENTS.md` contains only cross-tool project invariants and a short context router.
- `.claude/CLAUDE.md` imports `AGENTS.md`; it does not copy the project rules. Keeping the
  wrapper under `.claude/` avoids making it another root instruction surface for other tools.
- `.cursor/rules/*.mdc` adds only the two areas that benefit from Cursor's path-scoped rules.
- `inventory.json` makes startup budgets, imports, and Cursor scopes reviewable.

Run `just prompt-check` after changing these surfaces. The check rejects unregistered Cursor
rules, duplicate or unsupported frontmatter, eager startup imports outside the inventory,
startup budget overflow, symlinked inventory, prompt files or imports, and scope drift. It is
read-only.

The design follows the official guidance current on 2026-09-23:

- OpenAI: keep `AGENTS.md` current, use progressive disclosure, and read only what the task
  needs: <https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra>
- Cursor: use scoped `.cursor/rules/*.mdc` files and references instead of copied context:
  <https://cursor.com/docs/rules>
- Claude Code: keep startup memory concise, import shared instructions, and use path-scoped
  `.claude/rules/` for conditional context: <https://code.claude.com/docs/en/memory>

Prompt files guide agent behavior; they are not an enforcement boundary for runtime, security,
or release policy. Those remain owned by code, hooks, and verification gates.
