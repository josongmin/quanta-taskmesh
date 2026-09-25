//! Validate the exact scenario bytes consumed by the host receipt checker.

use std::env;
use std::fs::File;
use std::io::{self, BufReader, Read};

use taskmesh_bench::host_load::RawHostRun;
use taskmesh_bench::host_scenarios::HostScenario;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let raw_path = match args.next() {
        None => None,
        Some(flag) if flag == "--raw" => Some(args.next().ok_or("--raw requires a path")?),
        _ => return Err("usage: host_scenario_validate [--raw RAW_FILE]".into()),
    };
    if args.next().is_some() {
        return Err("usage: host_scenario_validate [--raw RAW_FILE]".into());
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
    let _runtime = scenario.build_runtime()?;
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
