## Dependency Policy

Runtime dependencies are declared in each crate's `Cargo.toml` and audited by
`cargo deny check --config config/deny.toml` plus semgrep import rules.

**Allowed** (examples):

- Serialization: `serde`, `serde_json`
- CLI: `clap`
- Errors: `thiserror`, `anyhow` (prefer typed errors in domain code)
- Regex, datetime parsing, test tempdirs, `proptest`
- Workspace path crates (`logq-value`, `logq-segment`, …) per the edge graph

**Forbidden** (graph-level deny + semgrep):

- Lucene, Tantivy, Bleve, Elasticsearch/OpenSearch, Meilisearch, Quickwit, etc.
- Embedded DB/KV backends as a ready-made index (`rusqlite`, `rocksdb`, `sled`, …)
- Shelling out to `grep`/`rg`/`awk`/`sed` for query execution

**Use with care** (justify in `DESIGN.md` if you add them):

- Heavy async runtimes or networking stacks unrelated to local log search
- Crates that pull in forbidden transitive search-engine dependencies

Pin shared versions in the root `[workspace.dependencies]`. New runtime deps
must pass `just deny` and stay inside the allowed architecture edges.
