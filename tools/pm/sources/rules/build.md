## Build & Front Door

`just` is the canonical front door for build, test, and lint. Direct `cargo` is
fine for local iteration; closeout reports cite `just` commands.

Common commands:

```bash
just setup-rust       # install cargo-nextest if missing (one-time)
just fmt              # rustfmt (strict: rustfmt.toml)
just check            # cargo check --workspace
just clippy           # pedantic + nursery; production pass adds restriction lints
just test-fast        # nextest default profile (.config/nextest.toml)
just test-ci          # nextest ci profile (gate)
just test-tdd         # nextest tdd profile (fail-fast, minimal status noise)
just test-architecture # crate boundary checker
just py-lint          # ruff over tools/
just py-test          # pytest over tools/
just pm-sync          # regenerate agent docs
just pm-lint          # fail on generated doc drift
just gate             # canonical ship check
```

Rust tests use **cargo-nextest only** (`.config/nextest.toml`, `[profile.*]`
in `Cargo.toml`). Prefer `just test-fast` / `just test-ci`, or `cargo nt`
(alias). The built-in libtest runner is not used; gate scripts enforce nextest.

`just gate` runs: release build, fmt check, clippy (two passes), nextest ci,
`cargo deny`, semgrep, Python lint/tests, architecture checker, test budgets,
and `pm lint`.

Rust toolchain is pinned in `rust-toolchain.toml` (1.95.0). Clippy policy lives
in `config/clippy.toml`; production-only denies in `config/clippy-restrict.txt`.
Python tooling is managed via `uv` (`pyproject.toml` + `uv.lock`).

Sample data (gitignored):

```bash
just generate-logs
# or: python3 tools/generate_logs.py --size 500MB --output ./sample_logs/ --seed 42
```

CLI entrypoint:

```bash
cargo build --release --bin logq
./target/release/logq ingest ./sample_logs/
./target/release/logq search "ERROR"
```
