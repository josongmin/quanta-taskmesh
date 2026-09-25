//! Validate the exact scenario bytes consumed by the host receipt checker.

use std::env;
use std::fs::File;
use std::io::{self, BufReader, Read};

use taskmesh_bench::host_load::{RawHostRun, ResolvedHostTopology};
use taskmesh_bench::host_scenarios::HostScenario;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let mut raw_path = None;
    let mut topology_path = None;
    let mut emit_topology = false;
    while let Some(flag) = args.next() {
        if flag == "--raw" && raw_path.is_none() {
            raw_path = Some(args.next().ok_or("--raw requires a path")?);
        } else if flag == "--topology" && topology_path.is_none() {
            topology_path = Some(args.next().ok_or("--topology requires a path")?);
        } else if flag == "--emit-topology" && !emit_topology {
            emit_topology = true;
        } else {
            return Err("usage: host_scenario_validate [--raw RAW_FILE] [--topology TOPOLOGY_FILE] [--emit-topology]".into());
        }
    }
    if emit_topology && (raw_path.is_some() || topology_path.is_some()) {
        return Err("--emit-topology cannot be combined with artifact validation".into());
    }
    let mut bytes = Vec::new();
    io::stdin()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("scenario exceeds 16 MiB validation input limit".into());
    }
    let scenario = HostScenario::from_json(&bytes)?;
    // Builder policy validation is part of scenario preflight, before timing.
    let runtime = scenario.build_runtime()?;
    let resolved_topology = ResolvedHostTopology::from_runtime(&runtime);
    if emit_topology {
        println!("{}", serde_json::to_string(&resolved_topology)?);
        return Ok(());
    }
    if let Some(path) = topology_path {
        let file = File::open(path)?;
        if file.metadata()?.len() > 16 * 1024 * 1024 {
            return Err("topology artifact exceeds 16 MiB validation input limit".into());
        }
        let topology: ResolvedHostTopology = serde_json::from_reader(BufReader::new(file))?;
        if topology != resolved_topology {
            return Err("topology artifact differs from freshly resolved runtime".into());
        }
    }
    if let Some(path) = raw_path {
        let file = File::open(path)?;
        if file.metadata()?.len() > 1024 * 1024 * 1024 {
            return Err("raw artifact exceeds 1 GiB validation input limit".into());
        }
        let raw: RawHostRun = serde_json::from_reader(BufReader::new(file))?;
        raw.validate_against(&scenario)?;
    }
    println!("SCENARIO_VALID id={}", scenario.id);
    Ok(())
}
