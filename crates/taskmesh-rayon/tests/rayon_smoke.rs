//! T09: the Rayon adapter compiles, runs work, and honors the topology clamp.

use std::sync::mpsc;

use taskmesh_contract::{CpuExecutor, TopologyConfig};
use taskmesh_rayon::RayonCpuExecutor;

#[test]
fn executor_runs_cpu_work() {
    let executor = RayonCpuExecutor::new(2);
    assert_eq!(executor.worker_count(), 2);

    let (tx, rx) = mpsc::channel();
    executor.spawn(Box::new(move || {
        let _ = tx.send(6 * 7);
    }));
    assert_eq!(rx.recv().unwrap(), 42);
}

#[test]
fn worker_count_respects_topology_clamp() {
    // Fixed mode is taken verbatim (clamped to >= 1).
    let fixed = RayonCpuExecutor::from_topology(&TopologyConfig::new().cpu_fixed(4));
    assert_eq!(fixed.worker_count(), 4);

    // max_workers clamps auto sizing down.
    let clamped = RayonCpuExecutor::from_topology(&TopologyConfig::new().cpu_auto().max_workers(1));
    assert_eq!(clamped.worker_count(), 1);

    // min_workers raises a tiny auto budget.
    let raised = RayonCpuExecutor::from_topology(
        &TopologyConfig::new()
            .cpu_auto()
            .reserve_cores(1024)
            .min_workers(2),
    );
    assert_eq!(raised.worker_count(), 2);
}
