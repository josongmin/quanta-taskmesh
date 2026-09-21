//! T09: the Rayon adapter compiles, runs work, and honors the topology clamp.

use std::sync::mpsc;

use taskmesh_contract::{CpuExecutor, TopologyConfig, TopologyError, PHYSICAL_CPU};
use taskmesh_rayon::{RayonBuildError, RayonCpuExecutor};

#[test]
fn executor_runs_cpu_work() {
    let executor = RayonCpuExecutor::try_new(2).expect("pool builds");
    assert_eq!(executor.worker_count(), 2);

    let (tx, rx) = mpsc::channel();
    executor.spawn(Box::new(move || {
        // The receiver is alive for the whole test; a failed send would mean the
        // harness itself broke, so it is worth asserting rather than discarding.
        tx.send(6 * 7).expect("the test still holds the receiver");
    }));
    assert_eq!(rx.recv().unwrap(), 42);
}

#[test]
fn worker_count_respects_topology_clamp() {
    // Fixed mode is taken verbatim (clamped to >= 1).
    let fixed = RayonCpuExecutor::try_from_topology(&TopologyConfig::new().cpu_fixed(4)).unwrap();
    assert_eq!(fixed.worker_count(), 4);

    // max_workers clamps auto sizing down.
    let clamped =
        RayonCpuExecutor::try_from_topology(&TopologyConfig::new().cpu_auto().max_workers(1))
            .unwrap();
    assert_eq!(clamped.worker_count(), 1);

    // min_workers raises a tiny auto budget.
    let raised = RayonCpuExecutor::try_from_topology(
        &TopologyConfig::new()
            .cpu_auto()
            .reserve_cores(1024)
            .min_workers(2),
    )
    .unwrap();
    assert_eq!(raised.worker_count(), 2);
}

#[test]
fn the_adapter_declares_what_it_can_honestly_promise() {
    // D05: a pool this adapter built is exclusive and its declared worker
    // count is the pool's real thread count.
    let owned = RayonCpuExecutor::try_new(3).unwrap();
    let declared = owned.capabilities();
    assert_eq!(declared.declared_workers, Some(3));
    assert!(
        declared.exclusive_pool,
        "a pool the adapter built is its own"
    );
    assert!(
        declared.nonblocking_submit,
        "ThreadPool::spawn never runs inline"
    );

    // A pool handed in from outside may have other users: the adapter says
    // *shared*, so the host never claims to bound work it cannot see. The
    // worker count is still the pool's real count, not a guess.
    let pool = std::sync::Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .expect("pool builds"),
    );
    let shared = RayonCpuExecutor::with_pool(pool);
    let declared = shared.capabilities();
    assert_eq!(declared.declared_workers, Some(2));
    assert!(!declared.exclusive_pool);
    assert!(declared.nonblocking_submit);
    assert_eq!(declared.physical_domain, Some(PHYSICAL_CPU));
}

#[test]
fn impossible_construction_is_typed_and_never_clamped() {
    let Err(zero_workers) = RayonCpuExecutor::try_new(0) else {
        panic!("zero workers must be rejected")
    };
    assert!(matches!(zero_workers, RayonBuildError::ZeroWorkers));
    assert_eq!(
        zero_workers.to_string(),
        "rayon worker count must be nonzero",
        "the public typed error must retain an actionable diagnostic"
    );
    assert!(matches!(
        RayonCpuExecutor::try_from_topology(&TopologyConfig::new().cpu_fixed(0)),
        Err(RayonBuildError::InvalidTopology(
            TopologyError::ZeroFixedCpuWorkers
        ))
    ));
}
