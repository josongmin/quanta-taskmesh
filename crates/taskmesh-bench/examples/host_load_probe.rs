//! Diagnostic public-host run. The emitted raw rows require `host_perf.py`
//! validation and a quiet-host receipt before they can support a performance claim.

use std::env;
use std::fs;
use std::path::PathBuf;

use taskmesh_bench::host_load::{run_host_scenario_with_fault, HostHarnessFault};
use taskmesh_bench::host_scenarios::HostScenario;

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
    let fault =
        match args.next() {
            None => None,
            Some(flag) if flag == "--inject-producer-before-first-offer" => {
                Some(HostHarnessFault::ProducerBeforeOffer(0))
            }
            _ => return Err(
                "usage: host_load_probe SCENARIO RAW_OUT [--inject-producer-before-first-offer]"
                    .into(),
            ),
        };
    if args.next().is_some() {
        return Err(
            "usage: host_load_probe SCENARIO RAW_OUT [--inject-producer-before-first-offer]".into(),
        );
    }
    if output_path.exists() {
        return Err(format!("refusing to overwrite {}", output_path.display()).into());
    }
    let bytes = fs::read(&scenario_path)?;
    let scenario = HostScenario::from_json(&bytes)?;
    let run = run_host_scenario_with_fault(&scenario, fault).await?;
    let raw = serde_json::to_vec_pretty(&run)?;
    let temporary = output_path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, raw)?;
    fs::rename(&temporary, &output_path)?;
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
