//! The consumer-facing docs compile.
//!
//! `build.rs` extracts every ```rust block from `README.md`,
//! `docs/taskmesh-external-interface.md` and `CHANGELOG.md` and wraps each one,
//! verbatim, in the scaffold its fence attribute names (see `build.rs`),
//! supplying the names the fragments assume. This crate has
//! no runtime code of its own; its only job is that `cargo test --workspace`
//! refuses to pass while a documented example names a facade item that does not
//! exist or calls one with the wrong shape (round-3A found three such examples
//! by hand; this keeps them from coming back).
//!
//! The scaffold is deliberately generous with `allow`s: the fragments are
//! written for readers, not for clippy. What is *not* allowed is a type error.

#![cfg(test)]

/// The ambient names the README fragments assume. Adding a new fragment that
/// needs a new placeholder means adding it here — explicitly, next to the
/// others — rather than weakening the check.
#[allow(dead_code, clippy::unused_async, clippy::missing_errors_doc)]
mod prelude {
    pub use std::collections::BTreeMap;
    pub use std::sync::Arc;
    pub use std::time::Duration;

    pub use taskmesh::ext::{CpuExecutor, ExecutorCapabilities, Governor, PermitId, PolicySet};
    pub use taskmesh::*;

    /// A parsed document (README §3: `run_blocking` result type).
    #[derive(Debug, Default)]
    pub struct Doc;

    /// A ranked search hit (README §3: `run_cpu` result type).
    #[derive(Debug, Default)]
    pub struct Hit;

    /// The application's parse error (README §3 / §6).
    #[derive(Debug)]
    pub struct ParseError;

    impl std::fmt::Display for ParseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("parse error")
        }
    }

    impl std::error::Error for ParseError {}

    /// The application's own error type (README §3 / §6).
    #[derive(Debug)]
    pub struct MyError;

    impl std::fmt::Display for MyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("my error")
        }
    }

    impl std::error::Error for MyError {}

    pub async fn load() -> Result<String, std::io::Error> {
        Ok(String::new())
    }

    pub fn parse_sync() -> Doc {
        Doc
    }

    pub fn rank() -> Vec<Hit> {
        Vec::new()
    }

    pub async fn do_local() -> u32 {
        0
    }

    pub async fn fetch() -> u32 {
        0
    }

    /// What a consumer does on a retryable worker failure (CHANGELOG
    /// `GovernorError` migration arms).
    pub fn retry() {}

    /// Runs the job under a lease the embedder holds (CHANGELOG `LeaseToken`
    /// migration).
    pub fn run(job: fn() -> Result<Doc, ParseError>) {
        let _ = job;
    }

    /// Records a child's lineage point (CHANGELOG `TaskScope::Child` migration).
    pub fn attribute(parent_stage: &TaskStage) {
        let _ = parent_stage;
    }

    /// The application's own CPU pool behind a custom `CpuExecutor` (CHANGELOG
    /// `ExecutorCapabilities` migration). The example writes the `impl`
    /// itself, so only the struct and the pool it wraps live here.
    pub struct MyPool {
        pub pool: MyThreadPool,
    }

    impl MyPool {
        pub fn with_threads(threads: usize) -> Self {
            Self {
                pool: MyThreadPool { threads },
            }
        }
    }

    pub struct MyThreadPool {
        threads: usize,
    }

    impl MyThreadPool {
        pub fn execute(&self, work: Box<dyn FnOnce() + Send + 'static>) {
            work();
        }

        pub fn threads(&self) -> usize {
            self.threads
        }
    }
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/blocks.rs"));
}

/// The docs whose examples are consumer-facing promises. Kept here, separately
/// from the build script's own list, on purpose: the tests below check the
/// extractor against *this* list, so a doc dropped from `build.rs` is caught
/// rather than silently no longer checked.
const DOCS: &[&str] = &[
    "README.md",
    "docs/taskmesh-external-interface.md",
    "CHANGELOG.md",
];

/// The one doc allowed to carry ```rust,ignore fences: its migration entries
/// quote the *previous* release's API, which must not compile against this one.
const MAY_IGNORE: &str = "CHANGELOG.md";

/// The line a ```rust,ignore fence in the CHANGELOG has to open with. An
/// ignored block is unchecked code; the only unchecked code the docs may carry
/// is code that documents an older release, and it says so on its first line.
const PREVIOUS_RELEASE_MARKER: &str = "// 0.1.0";

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

/// Every ```rust fence in `markdown` as `(1-based line, info string, first
/// non-empty line of the block)`, found by a scan independent of `build.rs`.
fn rust_fences(markdown: &str) -> Vec<(usize, String, String)> {
    let mut fences = Vec::new();
    let mut open: Option<(usize, String, Option<String>)> = None;
    for (offset, line) in markdown.lines().enumerate() {
        match open.as_mut() {
            None => {
                if let Some(info) = line.strip_prefix("```") {
                    open = Some((offset + 1, info.trim().to_owned(), None));
                }
            }
            Some((line_no, info, first)) => {
                if line.trim_end() == "```" {
                    let is_rust = info == "rust" || info.starts_with("rust,");
                    if is_rust {
                        fences.push((*line_no, info.clone(), first.clone().unwrap_or_default()));
                    }
                    open = None;
                } else if first.is_none() && !line.trim().is_empty() {
                    *first = Some(line.to_owned());
                }
            }
        }
    }
    assert!(open.is_none(), "unterminated code fence");
    fences
}

/// The build script's extraction is compared against an independent fence
/// count taken here at test time, so an extractor that silently found nothing
/// (a changed fence style, a moved file, a doc dropped from its list) cannot
/// pass as "all examples compile". Every fence that is not `rust,ignore` must
/// have produced exactly one compiled block, at the fence's own line.
#[test]
fn every_rust_fence_in_the_docs_was_extracted() {
    let repo = repo_root();
    for source in DOCS {
        let Some((_, extracted)) = generated::SOURCES.iter().find(|(name, _)| name == source)
        else {
            panic!("{source}: not extracted by build.rs — a documented example set is unchecked");
        };
        let markdown = std::fs::read_to_string(repo.join(source)).expect("doc is readable");
        let fences = rust_fences(&markdown);
        assert!(
            !fences.is_empty(),
            "{source}: no ```rust fences — the check would be vacuous"
        );
        let compiled: Vec<usize> = fences
            .iter()
            .filter(|(_, info, _)| info != "rust,ignore")
            .map(|(line, _, _)| *line)
            .collect();
        assert_eq!(
            *extracted,
            compiled.len(),
            "{source}: build.rs extracted {extracted} blocks but the file has {} compiled ```rust fences",
            compiled.len()
        );
        let generated_lines: Vec<usize> = generated::BLOCKS
            .iter()
            .filter(|(name, _, _)| name == source)
            .map(|(_, line, _)| *line)
            .collect();
        assert_eq!(
            generated_lines, compiled,
            "{source}: the compiled blocks are not the fences at these lines"
        );
    }
}

/// `rust,ignore` is the one hole in the check, so it is fenced in: only the
/// CHANGELOG may use it, and only for a block that opens by naming the
/// previous release. A consumer-facing example that stops compiling cannot be
/// hidden by tagging it.
#[test]
fn an_ignored_fence_is_only_ever_a_previous_release_quotation() {
    let repo = repo_root();
    for source in DOCS {
        let markdown = std::fs::read_to_string(repo.join(source)).expect("doc is readable");
        for (line, info, first) in rust_fences(&markdown) {
            if info != "rust,ignore" {
                continue;
            }
            assert_eq!(
                *source, MAY_IGNORE,
                "{source}:{line}: ```rust,ignore is reserved for the CHANGELOG's previous-release quotations"
            );
            assert!(
                first.trim_start().starts_with(PREVIOUS_RELEASE_MARKER),
                "{source}:{line}: an ignored block must open with {PREVIOUS_RELEASE_MARKER:?} (it is unchecked code), got {first:?}"
            );
        }
    }
}

/// The fence attributes are exercised, not merely accepted: the CHANGELOG's
/// migration entries need every scaffold shape, so if one silently stopped
/// being used (a fragment re-tagged to `ignore`, an attribute dropped by a
/// format pass) the compiled surface would have shrunk without a failure.
#[test]
fn every_scaffold_shape_compiles_at_least_one_changelog_block() {
    for attribute in ["", "body", "arms", "builder"] {
        let count = generated::BLOCKS
            .iter()
            .filter(|(source, _, kind)| *source == MAY_IGNORE && *kind == attribute)
            .count();
        assert!(
            count > 0,
            "CHANGELOG.md: no compiled ```rust{}{attribute} block — that scaffold shape is unexercised",
            if attribute.is_empty() { "" } else { "," }
        );
    }
}
