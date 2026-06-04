## Claude Code Notes

- `CLAUDE.md` is generated from `tools/pm/sources/**`. Never hand-edit it; run
  `python3 tools/pm/pm.py sync` after source edits.
- Self-check against the Rule Cheatsheet before claiming a slice is done.
- Prefer explaining trade-offs in `DESIGN.md` (“observed X → chose Y”) over
  generic best-practice language.
- The interview may ask you to extend query syntax or handle a new broken-line
  case — keep parsers and query evaluation in separate modules.
