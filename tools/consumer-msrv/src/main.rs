//! Exercises the public surface a downstream consumer relies on, so that
//! "compiles on the declared MSRV" is a statement about the API people use and
//! not about an empty crate.

use std::sync::Arc;

use taskmesh::ext::{AdmissionDecision, ClaimOutcome, Governor, PolicySet, StageReleaseOutcome};
use taskmesh::{
    Builder, ClassPolicy, ExecutionPhase, MemoryReleasePolicy, OverflowPolicy, ResourceBudget,
    RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TopologyConfig,
};

fn build() -> taskmesh::TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(2).large_stack_slots(1))
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .memory_units(1)
                .memory_release_policy(MemoryReleasePolicy::OnStageBoundary)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("consumer configuration builds")
}

#[tokio::main]
async fn main() {
    let runtime = build();

    // The everyday driving port.
    let out: i32 = runtime
        .run_blocking(
            TaskSpec::blocking(TaskClass::new("retrieval")).operation("op"),
            || Ok::<_, ()>(7),
        )
        .await
        .expect("blocking work runs");
    assert_eq!(out, 7);

    let out: i32 = runtime
        .run_cpu_with(
            TaskSpec::cpu(TaskClass::new("retrieval")).operation("cpu"),
            SubmitOptions::unbounded(),
            || Ok::<_, ()>(8),
        )
        .await
        .expect("cpu work runs");
    assert_eq!(out, 8);

    let out: i32 = runtime
        .run_io(TaskSpec::io(TaskClass::new("retrieval")).operation("io"), async {
            Ok::<_, ()>(9)
        })
        .await
        .expect("io work runs");
    assert_eq!(out, 9);

    // The typed error split survives.
    let rejected = runtime
        .run_io(TaskSpec::io(TaskClass::new("ghost")).operation("x"), async {
            Ok::<i32, ()>(0)
        })
        .await
        .expect_err("unknown class rejects");
    assert!(matches!(rejected, RunError::Governor(_)));

    // Observation surface, including the exact-width fields.
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.conservation_violation(), None);
    let observed = &snapshot.classes[&TaskClass::new("retrieval")];
    assert_eq!(observed.inflight, 0);
    assert_eq!(observed.cpu_units_held, 0u128);
    assert!(snapshot.capabilities.contains_key("blocking"));

    // Direct engine embedding through `ext`.
    let governor = Governor::new(PolicySet::default(), Arc::new(taskmesh::ext::ManualClock::new(0)))
        .expect("default policy set is constructible");
    assert!(matches!(
        governor.admit(&TaskSpec::io(TaskClass::new("unknown")).operation("x")),
        AdmissionDecision::Rejected(_)
    ));
    assert_eq!(governor.claim(u64::MAX), ClaimOutcome::Invalid);
    assert_eq!(
        governor.release_stage_memory(u64::MAX, 1),
        StageReleaseOutcome::UnknownPermit
    );
    assert!(!governor.advance_phase(u64::MAX, ExecutionPhase::Running));

    println!("taskmesh-consumer-msrv ok");
}
