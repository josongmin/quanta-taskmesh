## Codex Notes

- Cite exact commands from `README.md` in closeout (`just gate`, ingest, search).
- Before locking design, run `head`/`wc`/`python3 -c` snippets on `sample_logs/`
  and record observations in `DESIGN.md`.
- Edit agent guidance in `tools/pm/sources/**` + `python3 tools/pm/pm.py sync`;
  never patch generated files.
- `MEASUREMENTS.md` is evidence, not aspiration — include command + dataset size.
- When deferring a Core/Stretch item, say why in `README.md` (time, risk, deps).
