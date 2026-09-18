//! The consumer-facing docs compile.
//!
//! `build.rs` extracts every ```rust block from `README.md` and
//! `docs/taskmesh-external-interface.md` and wraps each one, verbatim, in an
//! async scaffold that supplies the names the fragments assume. This crate has
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
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/blocks.rs"));
}

/// The docs whose examples are consumer-facing promises. Kept here, separately
/// from the build script's own list, on purpose: the test below checks the
/// extractor against *this* list, so a doc dropped from `build.rs` is caught
/// rather than silently no longer checked.
const DOCS: &[&str] = &["README.md", "docs/taskmesh-external-interface.md"];

/// The build script's extraction is compared against an independent fence
/// count taken here at test time, so an extractor that silently found nothing
/// (a changed fence style, a moved file, a doc dropped from its list) cannot
/// pass as "all examples compile".
#[test]
fn every_rust_fence_in_the_docs_was_extracted() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root");
    for source in DOCS {
        let Some((_, extracted)) = generated::SOURCES.iter().find(|(name, _)| name == source)
        else {
            panic!("{source}: not extracted by build.rs — a documented example set is unchecked");
        };
        let markdown = std::fs::read_to_string(repo.join(source)).expect("doc is readable");
        let fences = markdown
            .lines()
            .filter(|line| {
                let line = line.trim_end();
                line == "```rust" || line.starts_with("```rust,")
            })
            .count();
        assert!(
            fences > 0,
            "{source}: no ```rust fences — the check would be vacuous"
        );
        assert_eq!(
            *extracted, fences,
            "{source}: build.rs extracted {extracted} blocks but the file has {fences} ```rust fences"
        );
    }
}
