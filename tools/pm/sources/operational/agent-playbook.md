## Closeout Playbook

Every meaningful slice should close with:

1. The code change (smallest surface that proves the behavior).
2. Tests in the owning crate module (unit and/or integration as appropriate).
3. An ingest→search integration test if the slice crosses layers.
4. Doc updates if user-visible behavior or query syntax changed.
5. `MEASUREMENTS.md` update only if you changed ingest/query perf characteristics.

Before closeout, run:

```bash
# Regenerate agent docs if you edited tools/pm/sources/**
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint

uv run ruff check tools
uv run pytest tools -q
just test-ci
just gate
```

Manual smoke:

```bash
just generate-logs 100MB
cargo build --release --bin logq
./target/release/logq ingest ./sample_logs/
./target/release/logq search "ERROR AND service=payment"
./target/release/logq search "user_id=42" --since 1h
```

Closeout report format:

```text
Changed:
- ...

Verified:
- just gate / nextest …
- pm lint
- manual: logq ingest / logq search …

Not run:
- ...

Known risk:
- ...
```

Fail-closed principle: if `pm lint` fails, sync — do not patch generated files.
If tests fail, fix behavior; do not weaken assertions to greenwash.
