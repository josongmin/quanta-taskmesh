//! Create-only same-process stability diagnostics and complete-population replay.
use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use taskmesh_bench::host_stability::{
    run_stability, StabilityCycle, StabilityManifest, StabilitySummary,
};

fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err(format!("regular artifact required: {}", path.display()));
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    taskmesh_bench::artifact::write_new(path, &bytes).map_err(|e| e.to_string())
}

fn validate(directory: &Path, manifest: &StabilityManifest) -> Result<(), String> {
    manifest.validate()?;
    let cycles = directory.join("cycles");
    if !fs::symlink_metadata(&cycles)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return Err("regular cycle directory required".into());
    }
    let count = fs::read_dir(&cycles).map_err(|e| e.to_string())?.count();
    if count != manifest.cycles as usize {
        return Err("missing or extra cycle artifacts".into());
    }
    let summary: StabilitySummary = read(&directory.join("summary.json"))?;
    let topology = manifest.scenario.resolved_topology()?;
    let mut previous = summary.baseline.clone();
    for index in 0..manifest.cycles {
        let cycle: StabilityCycle = read(&cycles.join(format!("cycle-{index:06}.json")))?;
        cycle.validate_with_topology(manifest, &previous, index, &topology)?;
        previous = cycle.after_canaries;
    }
    summary.validate_with_topology(manifest, &previous, &topology)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    if let Err(error) = command().await {
        eprintln!("stability rejected: {error}");
        std::process::exit(1);
    }
}

async fn command() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 && args.len() != 4 {
        return Err(
            "usage: host_stability_probe check MANIFEST | run/validate MANIFEST DIRECTORY".into(),
        );
    }
    let manifest: StabilityManifest = read(Path::new(&args[2]))?;
    manifest.validate()?;
    match args[1].as_str() {
        "check" if args.len() == 3 => Ok(()),
        "validate" if args.len() == 4 => validate(Path::new(&args[3]), &manifest),
        "run" if args.len() == 4 => {
            let directory = Path::new(&args[3]);
            fs::create_dir(directory.join("cycles")).map_err(|e| e.to_string())?;
            let summary = run_stability(&manifest, |cycle| {
                write(
                    &directory
                        .join("cycles")
                        .join(format!("cycle-{:06}.json", cycle.index)),
                    cycle,
                )
            })
            .await?;
            write(&directory.join("summary.json"), &summary)?;
            // Online validation already checked every cycle and the summary.
            // Full artifact replay belongs to a separate supervised process.
            if let Some(reason) = summary.reason {
                Err(reason)
            } else {
                Ok(())
            }
        }
        _ => Err("invalid stability command".into()),
    }
}
