// Retain the actual soak test's assertions while injecting disabled class policy
// and proving that each previously discarded result is ClassDisabled.
// Production/test source is read only; generated code lives in Cargo OUT_DIR.
use std::{env, fs, path::PathBuf};

fn main() {
    let source_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../../../../crates/taskmesh/tests/e2e_chaos.rs");
    println!("cargo:rerun-if-changed={}", source_path.display());
    let source = fs::read_to_string(source_path).unwrap();
    let helpers_start = source.find("use std::sync::atomic").unwrap();
    let helpers_end = source.find("// ─").unwrap();
    let start = source
        .find("async fn mixed_substrate_soak_with_concurrent_sweeper()")
        .unwrap();
    let end = start + source[start..].find("\n// ─").unwrap();
    let original = &source[start..end];
    let mut body = original.to_owned();
    for old in [".max_inflight(8)", ".max_inflight(2)", ".max_inflight(1)"] {
        assert_eq!(body.matches(old).count(), 1, "source guard: {old}");
        body = body.replace(old, ".max_inflight(0)");
    }
    for old in [
        "let _ = rt\n",
        "let _ = rt.run_cpu",
        "let _ = rt.run_blocking",
    ] {
        assert_eq!(body.matches(old).count(), 1, "source guard: {old}");
        body = body.replace(old, &old.replace("let _", "let outcome"));
    }
    let result_end = ".await;\n                }";
    assert_eq!(body.matches(result_end).count(), 3);
    body = body.replace(
        result_end,
        ".await;\nassert!(matches!(outcome, Err(RunError::Governor(\
         GovernorError::Rejected(AdmissionVerdict::ClassDisabled)))));\n                }",
    );
    let control = original.replacen(
        "async fn mixed_substrate_soak_with_concurrent_sweeper()",
        "async fn unmodified_soak_control()",
        1,
    );
    let generated = format!(
        "{}\npub {}\npub {}",
        source[helpers_start..helpers_end]
            .replace("AtomicBool, AtomicUsize, Ordering", "AtomicBool, Ordering"),
        control,
        body
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("disabled_soak.rs"),
        generated,
    )
    .unwrap();
}
