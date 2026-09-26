//! Revalidate separate diagnostic raw schemas with the retained bench binary.

use std::env;
use std::fs;

use taskmesh_bench::closed_loop::{ClosedLoopRun, ClosedLoopScenario};
use taskmesh_bench::composite_host::{CompositeFailureRaw, CompositeRaw, CompositeScenario};
use taskmesh_bench::host_load::ResolvedHostTopology;
use taskmesh_bench::local_host::{LocalHostRun, LocalHostScenario};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let mode = args
        .next()
        .ok_or("usage: host_special_validate MODE SCENARIO RAW TOPOLOGY")?;
    let scenario = fs::read(args.next().ok_or("missing scenario path")?)?;
    let raw = fs::read(args.next().ok_or("missing raw path")?)?;
    let topology: ResolvedHostTopology =
        serde_json::from_slice(&fs::read(args.next().ok_or("missing topology path")?)?)?;
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }
    match mode.to_str().ok_or("invalid mode")? {
        "closed_loop" => {
            let scenario = ClosedLoopScenario::from_json(&scenario)?;
            let raw: ClosedLoopRun = serde_json::from_slice(&raw)?;
            raw.validate_against(&scenario)?;
            if topology != scenario.resolved_topology()? {
                return Err("closed-loop resolved topology differs".into());
            }
        }
        "local" => {
            let scenario = LocalHostScenario::from_json(&scenario)?;
            let raw: LocalHostRun = serde_json::from_slice(&raw)?;
            raw.validate_against(&scenario)?;
            if topology != scenario.resolved_topology()? {
                return Err("local resolved topology differs".into());
            }
        }
        "composite" => {
            let scenario = CompositeScenario::from_json(&scenario)?;
            let raw: CompositeRaw = serde_json::from_slice(&raw)?;
            raw.validate_against(&scenario)?;
            if topology != scenario.resolved_topology()? {
                return Err("composite resolved topology differs".into());
            }
        }
        "composite_invalid" => {
            let scenario = CompositeScenario::from_json(&scenario)?;
            let raw: CompositeFailureRaw = serde_json::from_slice(&raw)?;
            raw.validate_against(&scenario)?;
            if topology != scenario.resolved_topology()? {
                return Err("composite failure resolved topology differs".into());
            }
            println!("SPECIAL_INVALID_RAW_STRUCTURALLY_VALID performance=UNQUALIFIED");
            return Ok(());
        }
        _ => return Err("unsupported special diagnostic mode".into()),
    }
    println!("SPECIAL_STRUCTURALLY_VALID performance=UNQUALIFIED");
    Ok(())
}
