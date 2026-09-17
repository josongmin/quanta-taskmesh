// Compile the repository's actual IAI function bodies without the measurement
// attributes. This proves Rust ownership timing, not Callgrind instruction counts.
use std::{env, fs, path::PathBuf};

fn main() {
    let source_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../../../../crates/taskmesh-bench/benches/iai_governance.rs");
    println!("cargo:rerun-if-changed={}", source_path.display());
    let source = fs::read_to_string(&source_path).unwrap();
    let mut generated = String::from(
        "use std::hint::black_box;\nuse taskmesh::{ext::{Governor, AdmissionDecision}, TaskSpec};\n",
    );
    for name in ["admit_release", "admit_unknown_reject", "snapshot"] {
        let start = source.find(&format!("fn {name}(")).unwrap();
        let open = start + source[start..].find('{').unwrap();
        let mut depth = 0;
        let mut end = None;
        // These three current bodies contain no brace-bearing string literals.
        for (offset, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        generated.push_str("pub ");
        generated.push_str(&source[start..end.unwrap()]);
        generated.push('\n');
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("iai_bodies.rs"),
        generated,
    )
    .unwrap();
}
