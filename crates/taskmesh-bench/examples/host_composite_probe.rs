//! Caller-orchestrated H6 diagnostic. It never establishes a performance verdict.

use std::env;
use std::fs;
use std::path::PathBuf;
use taskmesh_bench::artifact::write_new;

use taskmesh_bench::composite_host::{run_composite_with_topology, CompositeScenario};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_composite_probe SCENARIO RAW TOPOLOGY")?,
    );
    let raw_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_composite_probe SCENARIO RAW TOPOLOGY")?,
    );
    let topology_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_composite_probe SCENARIO RAW TOPOLOGY")?,
    );
    if args.next().is_some() || raw_path == topology_path {
        return Err("usage: host_composite_probe SCENARIO RAW TOPOLOGY (distinct outputs)".into());
    }
    if raw_path.exists() || topology_path.exists() {
        return Err("composite raw and topology output paths must be fresh".into());
    }
    let scenario = CompositeScenario::from_json(&fs::read(&scenario_path)?)?;
    let (run, topology) = match run_composite_with_topology(&scenario).await {
        Ok(result) => result,
        Err(failure) => {
            if let (Some(raw), Some(topology)) = (&failure.raw, &failure.topology) {
                write_new(&raw_path, &serde_json::to_vec_pretty(raw)?)?;
                write_new(&topology_path, &serde_json::to_vec_pretty(topology)?)?;
            }
            return Err(failure.reason.into());
        }
    };
    write_new(&raw_path, &serde_json::to_vec_pretty(&run)?)?;
    write_new(&topology_path, &serde_json::to_vec_pretty(&topology)?)?;
    run.validate_against(&scenario)?;
    println!(
        "COMPOSITE_DIAGNOSTIC performance=UNQUALIFIED children={} keyed={} raw={}",
        run.children.len(),
        run.reduced_keys.len(),
        raw_path.display()
    );
    Ok(())
}
