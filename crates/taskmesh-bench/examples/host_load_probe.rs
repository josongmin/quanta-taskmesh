//! Diagnostic public-host run. The emitted raw rows require `host_perf.py`
//! validation and a quiet-host receipt before they can support a performance claim.

use std::env;
use std::fs;
use std::path::PathBuf;
use taskmesh_bench::artifact::write_new;

use taskmesh_bench::host_load::{run_host_scenario_with_topology, HostHarnessFault};
use taskmesh_bench::host_scenarios::HostScenario;
use taskmesh_bench::minimal_host::run_minimal_host_scenario;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_load_probe SCENARIO RAW_OUT")?,
    );
    let output_path = PathBuf::from(
        args.next()
            .ok_or("usage: host_load_probe SCENARIO RAW_OUT")?,
    );
    let mut fault = None;
    let mut topology_path = None;
    let mut recorder_minimal = false;
    while let Some(flag) = args.next() {
        if flag == "--inject-producer-before-first-offer" && fault.is_none() {
            fault = Some(HostHarnessFault::ProducerBeforeOffer(0));
        } else if flag == "--recorder-minimal" && !recorder_minimal {
            recorder_minimal = true;
        } else if flag == "--topology-out" && topology_path.is_none() {
            topology_path = Some(PathBuf::from(
                args.next().ok_or("--topology-out requires a path")?,
            ));
        } else {
            return Err("usage: host_load_probe SCENARIO RAW_OUT [--topology-out PATH] [--inject-producer-before-first-offer] [--recorder-minimal]".into());
        }
    }
    if topology_path
        .as_ref()
        .is_some_and(|path| path == &output_path)
    {
        return Err("raw and topology output paths must differ".into());
    }
    let bytes = fs::read(&scenario_path)?;
    let scenario = HostScenario::from_json(&bytes)?;
    if recorder_minimal {
        let (run, topology) = run_minimal_host_scenario(&scenario, fault).await?;
        write_new(&output_path, &serde_json::to_vec_pretty(&run)?)?;
        if let Some(path) = topology_path {
            write_new(&path, &serde_json::to_vec_pretty(&topology)?)?;
        }
        if let Err(error) = run.validate_against(&scenario) {
            return Err(
                format!("minimal raw {} is invalid: {error}", output_path.display()).into(),
            );
        }
        println!(
            "MINIMAL_HOST scenario={} intended={} submitted={} responded={} dropped={} latency=UNAVAILABLE raw={}",
            run.scenario_id,
            run.counts.intended,
            run.counts.submitted,
            run.counts.responded,
            run.counts.caller_dropped,
            output_path.display()
        );
        return Ok(());
    }
    let (run, topology) = run_host_scenario_with_topology(&scenario, fault).await?;
    let raw = serde_json::to_vec_pretty(&run)?;
    write_new(&output_path, &raw)?;
    if let Some(path) = topology_path {
        write_new(&path, &serde_json::to_vec_pretty(&topology)?)?;
    }
    if let Err(error) = run.validate_against(&scenario) {
        return Err(format!("raw artifact {} is invalid: {error}", output_path.display()).into());
    }
    println!(
        "scenario={} intended={} submitted={} responded={} dropped={} unanswered={} drain_ok={} raw={}",
        run.scenario_id,
        run.settlement.intended,
        run.settlement.submitted,
        run.settlement.responded,
        run.settlement.caller_dropped,
        run.settlement.unanswered_at_settlement,
        run.drain_ok,
        output_path.display()
    );
    Ok(())
}
