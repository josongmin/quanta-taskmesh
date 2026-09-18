//! Extract every compiled ```rust block from the consumer-facing docs into a
//! Rust source file the crate compiles under `cfg(test)`.
//!
//! The docs' examples are fragments: they assume an ambient `runtime`, a
//! `spec`, a `job`, a `governor`, and application-side types (`Doc`, `MyError`,
//! `MyPool`, `load()`). Each block is therefore wrapped in one async scaffold
//! function whose parameters and prelude supply those names (see
//! `src/lib.rs::prelude`), and compiled — never run. A fragment that names a
//! facade item that no longer exists, or calls it with the wrong shape, fails
//! `cargo test --workspace`.
//!
//! Blocks are taken verbatim: nothing is rewritten, and no hidden `# ` lines are
//! required in the Markdown, so the docs stay readable on GitHub. What a block
//! *is* — a statement sequence, a complete body, match arms, a builder chain —
//! is declared on its fence instead, rustdoc-style (`rust,ignore`,
//! `rust,no_run`), by an attribute after the comma:
//!
//! | fence           | scaffold                                                         |
//! |-----------------|------------------------------------------------------------------|
//! | ```rust         | statements in an `async fn -> Result<(), Box<dyn Error>>`; `?` works |
//! | ```rust,body    | a complete `async move { … }` body: the block's own `return`s and tail expression decide its result type |
//! | ```rust,arms    | match arms over `runtime.run_blocking(spec, job).await`; the scaffold adds the `match` and a trailing `_ => {}` |
//! | ```rust,builder | a method chain continued from `Builder::new()`                    |
//! | ```rust,ignore  | **not compiled** and not counted (rendered as Rust on GitHub) — for code that names a *previous* release's API, e.g. the `// 0.1.0` halves of the CHANGELOG migration entries |
//!
//! Any other attribute is a build error: an unknown attribute silently treated
//! as the default would let a typo hide an unchecked example.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Docs whose ```rust blocks are consumer-facing promises.
const SOURCES: &[&str] = &[
    "README.md",
    "docs/taskmesh-external-interface.md",
    "CHANGELOG.md",
];

/// How a block is scaffolded, from its fence attribute. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Statements,
    Body,
    Arms,
    Builder,
}

impl Kind {
    /// The attribute spelling on the fence (`rust,<attribute>`); `rust` alone
    /// is `Statements`.
    fn attribute(self) -> &'static str {
        match self {
            Self::Statements => "",
            Self::Body => "body",
            Self::Arms => "arms",
            Self::Builder => "builder",
        }
    }
}

/// What an opening fence's info string means to the extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fence {
    /// A compiled example.
    Compiled(Kind),
    /// ```rust,ignore: Rust for the reader, invisible to the compiler.
    Ignored,
    /// Not Rust (`toml`, `text`, `sh`, unlabelled): tracked only so its closing
    /// ``` is not mistaken for an opening one.
    Other,
}

/// Classify an opening fence's info string. Only the exact `rust` info string,
/// optionally with one attribute after a comma, is a Rust fence.
fn classify(info: &str) -> Result<Fence, String> {
    let info = info.trim();
    if info == "rust" {
        return Ok(Fence::Compiled(Kind::Statements));
    }
    let Some(attribute) = info.strip_prefix("rust,") else {
        return Ok(Fence::Other);
    };
    match attribute.trim() {
        "ignore" => Ok(Fence::Ignored),
        "body" => Ok(Fence::Compiled(Kind::Body)),
        "arms" => Ok(Fence::Compiled(Kind::Arms)),
        "builder" => Ok(Fence::Compiled(Kind::Builder)),
        other => Err(format!(
            "unknown ```rust fence attribute {other:?} (known: ignore, body, arms, builder)"
        )),
    }
}

struct Block {
    source: &'static str,
    index: usize,
    /// 1-based line of the opening fence in the source file.
    line: usize,
    kind: Kind,
    code: String,
}

/// An open code fence while scanning: only compiled `rust` fences collect
/// lines, but every fence is tracked so its closing ``` is not mistaken for an
/// opening one.
struct OpenFence<'a> {
    /// The opening fence's line (1-based) and kind; `None` for a fence the
    /// extractor does not compile (non-Rust, or `rust,ignore`).
    compiled: Option<(usize, Kind)>,
    lines: Vec<&'a str>,
}

/// Compiled ```rust blocks, in document order. A ```rust,ignore fence is
/// skipped (its lines are not collected and it produces no block); every
/// other `rust,<attribute>` fence is compiled with the scaffold the attribute
/// names, and an unknown attribute is an error.
fn rust_blocks(source: &'static str, markdown: &str) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut open: Option<OpenFence<'_>> = None;
    for (offset, line) in markdown.lines().enumerate() {
        match open.as_mut() {
            None => {
                if let Some(info) = line.strip_prefix("```") {
                    let fence = classify(info)
                        .map_err(|error| format!("{source}:{}: {error}", offset + 1))?;
                    open = Some(OpenFence {
                        compiled: match fence {
                            Fence::Compiled(kind) => Some((offset + 1, kind)),
                            Fence::Ignored | Fence::Other => None,
                        },
                        lines: Vec::new(),
                    });
                }
            }
            Some(fence) => {
                if line.trim_end() == "```" {
                    if let Some((line_no, kind)) = fence.compiled {
                        blocks.push(Block {
                            source,
                            index: blocks.len(),
                            line: line_no,
                            kind,
                            code: format!("{}\n", fence.lines.join("\n")),
                        });
                    }
                    open = None;
                } else {
                    fence.lines.push(line);
                }
            }
        }
    }
    if open.is_some() {
        return Err(format!("{source}: unterminated code fence"));
    }
    Ok(blocks)
}

fn module_name(block: &Block) -> String {
    let stem = Path::new(block.source)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    format!("{}_block_{}", stem.to_ascii_lowercase(), block.index)
}

/// The scaffold function's parameters: every ambient *value* a fragment may
/// name. Types and functions come from the prelude; values are parameters so
/// that nothing ever has to be constructed (the scaffold is compiled, not
/// called). A new fragment that needs a new ambient value gets it here.
const PARAMETERS: &str = "\
        runtime: &TokioRuntime,
        spec: TaskSpec,
        job: fn() -> Result<Doc, ParseError>,
        governor: &Governor,
        permit: PermitId,
        epoch: u64,
        measured_bytes: u64,
        snapshot: Snapshot,
        class: TaskClass,
        input: &str,
        available: usize,
        policy: PolicySet,
        resources: ResourceBudget,
        classes: BTreeMap<TaskClass, ClassPolicy>,
        my_records: Vec<SubstrateRecord>,
        limits: BTreeMap<String, u32>,
        d: Duration,
        fut: std::future::Ready<Result<u32, MyError>>,
";

/// Push `code` indented by `indent`, indenting non-empty lines only so the
/// generated file stays free of trailing whitespace.
fn push_indented(out: &mut String, code: &str, indent: &str) {
    for line in code.lines() {
        if !line.is_empty() {
            out.push_str(indent);
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn render(blocks: &[Block]) -> String {
    let mut out = String::new();
    out.push_str("// @generated by tools/doc-examples/build.rs — do not edit.\n");
    out.push_str("pub const SOURCES: &[(&str, usize)] = &[\n");
    for source in SOURCES {
        let count = blocks.iter().filter(|b| b.source == *source).count();
        out.push_str(&format!("    ({source:?}, {count}),\n"));
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Every compiled block: (source, 1-based line of its opening fence, fence attribute).\n",
    );
    out.push_str("pub const BLOCKS: &[(&str, usize, &str)] = &[\n");
    for block in blocks {
        out.push_str(&format!(
            "    ({:?}, {}, {:?}),\n",
            block.source,
            block.line,
            block.kind.attribute()
        ));
    }
    out.push_str("];\n\n");
    for block in blocks {
        let name = module_name(block);
        let fence = if block.kind == Kind::Statements {
            "```rust".to_owned()
        } else {
            format!("```rust,{}", block.kind.attribute())
        };
        out.push_str(&format!(
            "/// {}:{} — {fence} block #{}, verbatim.\n",
            block.source, block.line, block.index
        ));
        out.push_str(
            "#[allow(unused, dead_code, unreachable_code, unreachable_patterns, non_local_definitions, clippy::all, clippy::pedantic, clippy::nursery)]\n",
        );
        out.push_str(&format!("pub mod {name} {{\n"));
        out.push_str("    use crate::prelude::*;\n\n");
        out.push_str("    pub async fn block(\n");
        out.push_str(PARAMETERS);
        out.push_str("    ) -> Result<(), Box<dyn std::error::Error>> {\n");
        match block.kind {
            Kind::Statements => {
                push_indented(&mut out, &block.code, "        ");
            }
            Kind::Body => {
                out.push_str("        let _ = async move {\n");
                push_indented(&mut out, &block.code, "            ");
                out.push_str("        };\n");
            }
            Kind::Arms => {
                out.push_str("        match runtime.run_blocking(spec, job).await {\n");
                push_indented(&mut out, &block.code, "            ");
                out.push_str("            _ => {}\n        }\n");
            }
            Kind::Builder => {
                out.push_str("        let _ = Builder::new()\n");
                push_indented(&mut out, &block.code, "            ");
                out.push_str("        ;\n");
            }
        }
        out.push_str("        Ok(())\n    }\n}\n\n");
    }
    out
}

fn main() -> Result<(), String> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo = manifest_dir
        .ancestors()
        .nth(2)
        .expect("tools/doc-examples sits two levels below the repository root");
    let mut blocks = Vec::new();
    for source in SOURCES {
        let path = repo.join(source);
        println!("cargo:rerun-if-changed={}", path.display());
        let markdown = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let found = rust_blocks(source, &markdown)?;
        if found.is_empty() {
            return Err(format!(
                "no compiled ```rust blocks found in {source}: the extraction is broken, not the docs"
            ));
        }
        blocks.extend(found);
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("blocks.rs"), render(&blocks))
        .map_err(|error| format!("cannot write blocks.rs: {error}"))?;
    Ok(())
}
