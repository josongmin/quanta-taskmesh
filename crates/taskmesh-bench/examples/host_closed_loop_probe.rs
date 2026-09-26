//! Diagnostic fixed-concurrency runner. Its completion samples are not
//! intended-arrival overload samples and have no performance gate.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use taskmesh_bench::closed_loop::{run_closed_loop_with_topology, ClosedLoopScenario};

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_closed_loop_probe SCENARIO RAW TOPOLOGY")?,
    );
    let raw_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_closed_loop_probe SCENARIO RAW TOPOLOGY")?,
    );
    let topology_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_closed_loop_probe SCENARIO RAW TOPOLOGY")?,
    );
    if args.next().is_some() || raw_path == topology_path {
        return Err(
            "usage: host_closed_loop_probe SCENARIO RAW TOPOLOGY (distinct outputs)".into(),
        );
    }
    if raw_path.exists() || topology_path.exists() {
        return Err("closed-loop raw and topology output paths must be fresh".into());
    }
    let scenario = ClosedLoopScenario::from_json(&fs::read(&scenario_path)?)?;
    let (run, topology) = run_closed_loop_with_topology(&scenario).await?;
    write_new(&raw_path, &serde_json::to_vec_pretty(&run)?)?;
    write_new(&topology_path, &serde_json::to_vec_pretty(&topology)?)?;
    run.validate_against(&scenario)?;
    println!(
        "CLOSED_LOOP_DIAGNOSTIC performance=UNQUALIFIED slots={} iterations={} completed={} raw={}",
        run.concurrency,
        run.iterations_per_slot,
        run.records.iter().filter(|row| row.is_some()).count(),
        raw_path.display()
    );
    Ok(())
}
