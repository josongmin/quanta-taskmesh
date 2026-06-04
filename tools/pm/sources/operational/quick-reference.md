## Quick Reference

Indexed log search over mixed-format synthetic logs.

Start here:

- Read `docs/00.requirements.md` end-to-end.
- Generate data: `just generate-logs` or `python3 tools/generate_logs.py --size 500MB --output ./sample_logs/ --seed 42`
- Skim all three files in `sample_logs/` before choosing index and query design.
- Architecture + lint cheatsheet: `.codex/CODEX-RULES.md`

Most-used commands:

```bash
just gate              # ship check (fmt, clippy, tests, semgrep, deny, …)
just clippy            # pedantic + production restriction lints
just test-architecture # crate boundary checker
just pm-sync && just pm-lint

cargo build --release --bin logq
./target/release/logq ingest ./sample_logs/
./target/release/logq search "ERROR"
```

Hard rules:

- Index-backed search — not `grep -r` as the engine.
- No external search-engine libraries.
- Deterministic persisted metadata and query output ordering.
- Never hand-edit generated agent docs — edit `tools/pm/sources/**` and sync.
