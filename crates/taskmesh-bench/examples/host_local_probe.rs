//! Caller-affine `!Send` diagnostic with a distinct raw schema.

use std::env;
use std::fs;
use std::path::PathBuf;
use taskmesh_bench::artifact::write_new;

use taskmesh_bench::local_host::{run_local_host_with_topology, LocalHostScenario};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_local_probe SCENARIO RAW TOPOLOGY")?,
    );
    let raw_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_local_probe SCENARIO RAW TOPOLOGY")?,
    );
    let topology_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_local_probe SCENARIO RAW TOPOLOGY")?,
    );
    if args.next().is_some() || raw_path == topology_path {
        return Err("usage: host_local_probe SCENARIO RAW TOPOLOGY (distinct outputs)".into());
    }
    if raw_path.exists() || topology_path.exists() {
        return Err("local raw and topology output paths must be fresh".into());
    }
    let scenario = LocalHostScenario::from_json(&fs::read(&scenario_path)?)?;
    let (run, topology) = run_local_host_with_topology(&scenario).await?;
    write_new(&raw_path, &serde_json::to_vec_pretty(&run)?)?;
    write_new(&topology_path, &serde_json::to_vec_pretty(&topology)?)?;
    run.validate_against(&scenario)?;
    println!(
        "LOCAL_HOST_DIAGNOSTIC performance=UNQUALIFIED intended={} submitted={} raw={}",
        run.records.len(),
        run.records
            .iter()
            .filter(|row| row.submitted_ns.is_some())
            .count(),
        raw_path.display()
    );
    Ok(())
}
