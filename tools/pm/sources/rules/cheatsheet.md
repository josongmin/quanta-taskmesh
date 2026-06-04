## Rule Cheatsheet (self-check before the gate)

Every row is enforced fail-closed — `just gate` rejects it. Fix the code, or
add a `// nosemgrep: <rule-id>` / `#[allow(<lint>)]` with a written reason.

| Don't write this | Write this instead | Enforced by |
| --- | --- | --- |
| `x.unwrap()` | `x.ok_or(Error::Missing)?` or `.expect("<invariant>")` | `logq-no-unwrap-in-production`, clippy `unwrap_used` |
| `panic!` / `unreachable!` / `todo!` / `unimplemented!` | return a typed error variant | semgrep + clippy |
| `.expect("")`, `.expect("TODO")` | `.expect("<why this cannot fail>")` | `logq-expect-needs-context` |
| `Err(_) => {}`, `let _ = fallible();`, `foo().ok();` | handle it: `?`, typed error, or record with context | semgrep |
| bare `#[allow(<lint>)]` / `// nosemgrep: <id>` (no reason) | append `-- reason: <…>` on the same line | `logq-no-unannotated-allow` |
| `arr[i]`, `&s[a..b]` | `arr.get(i)`, checked slicing | `logq-no-unchecked-indexing-in-engine`, clippy `indexing_slicing`, `string_slice` |
| `n as u32` (lossy) | `u32::try_from(n)?` | clippy cast lints |
| `a == b` on floats | epsilon or ordered comparison | clippy `float_cmp`, `lossy_float_literal` |
| `dbg!(...)` | remove it | `logq-no-dbg-macro`, clippy `dbg_macro` |
| `std::fs` in kernel / domain / ports | route through adapters or `logq-segment` | `logq-no-std-fs-in-engine` |
| `crate::adapters` in domain/ports | depend on port traits only | `logq-no-adapters-in-domain-or-ports` |
| `logq_ingest` in search (or vice versa) | compose at `logq-cli` | `logq-no-cross-feature-import` |
| `println!` / `eprintln!` / `process::exit` outside CLI | return a value/error to `logq-cli` | semgrep cli-boundary |
| `clap` outside `logq-cli` | parse at the CLI edge | `logq-no-clap-outside-cli` |
| `SystemTime::now` / `rand` / `Uuid::new_v4` outside `logq-clock` | inject via Clock port | semgrep determinism |
| `std::env::var` in engine layers | resolve at CLI, pass typed values inward | `logq-no-env-read-in-engine` |
| search-engine / DB-index crate | own index in `logq-segment` + features | `cargo deny`, semgrep |
| shell out to `grep`/`rg`/`awk`/`sed` | match in-process | `logq-no-shell-out-to-search-tools` |
| trait defs in `logq-segment` (kernel must stay concrete) | put ports in feature crate's `src/ports/` | `logq-no-traits-in-segment-crate` |
| `use logq_cli::*` from a non-cli crate | CLI is a sink, not a library | `logq-no-cli-import-from-other-crates` |
| `assert!(true)` / `assert_eq!(x, x)` placeholder in a test | assert the actual behavior | `logq-test-trivial-assert-true`, `logq-test-self-equality`, `logq-test-self-inequality` |
| `#[ignore]` without a reason | document why; consider `#[ignore = "<reason>"]` | `logq-ignore-needs-reason` |

Rule ids: `tools/semgrep/rules/**`. Clippy: `just clippy` (pedantic + restrict).
