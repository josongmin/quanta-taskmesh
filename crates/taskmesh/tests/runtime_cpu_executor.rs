//! T09: run_cpu rides the executor abstraction; the Rayon adapter plugs in and
//! the large-stack/blocking path stays off the shared CPU pool.

use std::sync::Arc;

use taskmesh::*;
use taskmesh_rayon::RayonCpuExecutor;

fn runtime_with_rayon(workers: usize) -> (TokioRuntime, usize) {
    let topology = TopologyConfig::new().cpu_fixed(workers);
    let rayon = RayonCpuExecutor::from_topology(&topology);
    let count = rayon.worker_count();
    let rt = Builder::new()
        .topology(topology)
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(rayon))
        .build()
        .unwrap();
    (rt, count)
}

#[tokio::test]
async fn cpu_work_executes_through_rayon_abstraction() {
    let (rt, workers) = runtime_with_rayon(3);
    assert_eq!(workers, 3, "worker count respects topology clamp");

    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let thread_name: String = rt
        .run_cpu(spec, || {
            Ok::<_, ()>(
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .await
        .expect("cpu work runs");
    assert!(
        thread_name.starts_with("taskmesh-cpu-"),
        "cpu work ran on the shared rayon pool, got {thread_name:?}"
    );
}

#[tokio::test]
async fn large_stack_blocking_path_avoids_shared_cpu_pool() {
    let (rt, _) = runtime_with_rayon(2);

    // A large-stack-classified job goes through run_blocking, NOT the CPU
    // executor, so it must not land on a "taskmesh-cpu-*" rayon worker.
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("big-stack");
    let thread_name: String = rt
        .run_blocking(spec, || {
            Ok::<_, ()>(
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .await
        .expect("blocking work runs");
    assert!(
        !thread_name.starts_with("taskmesh-cpu-"),
        "large-stack work must not be routed to the shared CPU pool, got {thread_name:?}"
    );
}

#[tokio::test]
async fn default_runtime_works_without_rayon() {
    // Without an injected executor, run_cpu still works (blocking-pool default).
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(10))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(4).cpu_units(1),
        )
        .build()
        .unwrap();
    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let out: i32 = rt.run_cpu(spec, || Ok::<_, ()>(5)).await.unwrap();
    assert_eq!(out, 5);
}
