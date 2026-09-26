//! Generator-only headroom control for a typed Taskmesh host scenario.

use std::env;
use std::fs;
use std::path::PathBuf;
use taskmesh_bench::artifact::write_new;

use taskmesh_bench::generator_calibration::run_generator_control_with_topology;
use taskmesh_bench::host_load::HostHarnessFault;
use taskmesh_bench::host_scenarios::HostScenario;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_generator_probe SCENARIO RAW_OUT")?,
    );
    let output_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_generator_probe SCENARIO RAW_OUT")?,
    );
    let mut fault = None;
    let mut topology_path = None;
    while let Some(flag) = args.next() {
        if flag == "--inject-producer-before-first-offer" && fault.is_none() {
            fault = Some(HostHarnessFault::ProducerBeforeOffer(0));
        } else if flag == "--topology-out" && topology_path.is_none() {
            topology_path = Some(PathBuf::from(
                args.next().ok_or("--topology-out requires a path")?,
            ));
        } else {
            return Err("usage: host_generator_probe SCENARIO RAW_OUT [--topology-out PATH] [--inject-producer-before-first-offer]".into());
        }
    }
    if topology_path
        .as_ref()
        .is_some_and(|path| path == &output_path)
    {
        return Err("raw and topology output paths must differ".into());
    }
    let scenario = HostScenario::from_json(&fs::read(&scenario_path)?)?;
    let (run, topology) = run_generator_control_with_topology(&scenario, fault).await?;
    write_new(&output_path, &serde_json::to_vec_pretty(&run)?)?;
    if let Some(path) = topology_path {
        write_new(&path, &serde_json::to_vec_pretty(&topology)?)?;
    }
    run.validate_against(&scenario)
        .map_err(|error| format!("generator raw artifact is invalid: {error}"))?;
    println!(
        "GENERATOR_CONTROL scenario={} intended={} submitted={} completed={} raw={}",
        run.scenario_id,
        run.records.len(),
        run.submitted,
        run.completed,
        output_path.display()
    );
    Ok(())
}
