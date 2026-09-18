//! A declared nested wait through the host (H16-010-A06, ADR 0003 D12).
//!
//! The engine-level rule is proved in
//! `crates/taskmesh-engine/tests/hardening_nested_wait.rs`; this is the shape
//! a consumer actually writes: a running job submits a child *and awaits it*.
//! With the child declared (`awaited_child_of`) the submission comes back at
//! once as `Rejected(NestedWaitCycle { .. })`, the parent can act on it, and
//! the runtime is idle afterwards. With the wait undeclared (`child_of`) the
//! engine infers nothing — the child queues, and the parent's own
//! `acquire_timeout` is what ends the wait, as a typed timeout.
//!
//! Every wait is bounded (5 s): a regression here is a named assertion, not a
//! hung test binary.

use std::time::Duration;

use taskmesh::*;

const HANG: Duration = Duration::from_secs(5);

async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: the caller was still waiting after {HANG:?}");
    };
    output
}

/// One slot in `c`, queueing: the shape in which a parent that awaits a
/// same-class child would deadlock.
fn runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(4)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

fn assert_idle(rt: &TokioRuntime, what: &str) {
    let snapshot = rt.snapshot();
    let c = &snapshot.classes[&TaskClass::new("c")];
    assert_eq!(
        (c.inflight, c.queued),
        (0, 0),
        "{what}: the runtime must be idle afterwards"
    );
    assert_eq!(snapshot.conservation_violation(), None, "{what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parent_that_awaits_a_declared_child_on_its_own_slot_is_told_so_at_once() {
    let rt = runtime();
    let inner = rt.clone();
    let parent = rt.run_io(
        TaskSpec::io(TaskClass::new("c")).operation("parent"),
        async move {
            // The parent holds the only slot and waits for a child that needs
            // it. Declared, so the engine answers instead of parking the child.
            let child = inner
                .run_io(
                    TaskSpec::io(TaskClass::new("c"))
                        .awaited_child_of("parent", TaskStage::new("child")),
                    async { Ok::<u32, ()>(1) },
                )
                .await;
            match child {
                Err(RunError::Governor(GovernorError::Rejected(
                    AdmissionVerdict::NestedWaitCycle { held_by_root },
                ))) => Ok::<HeldCapacity, ()>(held_by_root),
                other => {
                    panic!("a declared cycle must be refused as NestedWaitCycle, got {other:?}")
                }
            }
        },
    );
    let held = Box::pin(bounded("parent awaiting a declared cycle", parent))
        .await
        .expect("the parent completes with the child's verdict in hand");
    assert_eq!(
        held,
        HeldCapacity::ClassInflight {
            class: TaskClass::new("c")
        },
        "the verdict names the capacity the parent itself holds"
    );
    assert_idle(&rt, "after the refused cycle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_undeclared_child_is_not_inferred_and_the_parents_acquire_timeout_ends_the_wait() {
    // D12: lineage alone is not a wait. The child queues behind its parent;
    // nothing in the engine knows the parent is waiting, so the parent's own
    // acquisition budget is the only thing that ends it — typed, not hung.
    let rt = runtime();
    let inner = rt.clone();
    let parent = rt.run_io(
        TaskSpec::io(TaskClass::new("c")).operation("parent"),
        async move {
            let child = inner
                .run_io_with(
                    TaskSpec::io(TaskClass::new("c")).child_of("parent", TaskStage::new("child")),
                    SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(100)),
                    async { Ok::<u32, ()>(1) },
                )
                .await;
            match child {
                Err(RunError::Governor(GovernorError::Rejected(
                    AdmissionVerdict::PermitAcquireTimedOut { .. },
                ))) => Ok::<(), ()>(()),
                other => panic!(
                    "an undeclared child must queue (and here time out), not be refused as a cycle; got {other:?}"
                ),
            }
        },
    );
    Box::pin(bounded("parent awaiting an undeclared child", parent))
        .await
        .expect("the parent completes once its child's acquire budget expires");
    assert_idle(&rt, "after the timed-out undeclared wait");
}
