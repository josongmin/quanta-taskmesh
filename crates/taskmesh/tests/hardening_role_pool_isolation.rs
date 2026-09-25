//! B20: a saturated semantic role must not borrow spare physical-domain slots.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::{
    ext::{AbandonOutcome, AdmissionDecision, CapacityBlock, ReleaseOutcome},
    Builder, ClassPolicy, OverflowPolicy, PhysicalDomainMode, Runtime, TaskClass, TaskSpec,
    TokioRuntime, TopologyConfig,
};

const HANG: Duration = Duration::from_secs(5);
const STACK: u64 = 2 * 1024 * 1024;

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(1)
                .large_stack_slots(1)
                .local_runtime_slots(1)
                .shared_blocking_domain(PhysicalDomainMode::Fixed(2))
                .dedicated_domain(PhysicalDomainMode::Fixed(2)),
        )
        .class_policy(
            TaskClass::new("stack"),
            ClassPolicy::new()
                .max_inflight(3)
                .max_queue_depth(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(TaskClass::new("plain"), ClassPolicy::new().max_inflight(3))
        .build()
        .expect("bounded role pools inside larger physical domains")
}

#[test]
fn local_role_pool_saturation_names_local_runtime_and_leaves_physical_domains_free() {
    let rt = runtime();
    let governor = rt.governor();
    let holder =
        match governor.admit(&TaskSpec::local(TaskClass::new("stack")).operation("local-holder")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("local holder must admit: {other:?}"),
        };

    let ticket = match governor
        .admit(&TaskSpec::local(TaskClass::new("stack")).operation("local-follower"))
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("the local follower must wait on the role pool: {other:?}"),
    };
    assert_eq!(
        governor.pending_block_reason(ticket),
        Some(CapacityBlock::Capability)
    );
    let saturated = rt.snapshot();
    assert_eq!(saturated.capabilities["local_runtime"].in_use, 1);
    assert_eq!(saturated.capabilities["local_runtime"].limit, 1);
    assert_eq!(saturated.capabilities["physical.shared_blocking"].in_use, 0);
    assert_eq!(saturated.capabilities["physical.dedicated"].in_use, 0);

    let unrelated = match governor
        .admit(&TaskSpec::blocking(TaskClass::new("plain")).operation("unrelated-blocking"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("an unrelated physical domain remains usable: {other:?}"),
    };
    let with_unrelated = rt.snapshot();
    assert_eq!(with_unrelated.capabilities["local_runtime"].in_use, 1);
    assert_eq!(with_unrelated.capabilities["blocking"].in_use, 1);
    // Direct Governor admission charges semantic roles; host dispatch owns
    // physical-domain occupancy, which the async case below observes.
    assert_eq!(
        with_unrelated.capabilities["physical.shared_blocking"].in_use,
        0
    );
    assert_eq!(with_unrelated.capabilities["physical.dedicated"].in_use, 0);

    assert_eq!(governor.release(unrelated), ReleaseOutcome::Released);
    assert_eq!(governor.abandon(ticket), AbandonOutcome::Abandoned);
    assert_eq!(governor.release(holder), ReleaseOutcome::Released);
    let drained = rt.snapshot();
    assert_eq!(drained.capabilities["local_runtime"].in_use, 0);
    assert_eq!(drained.capabilities["physical.shared_blocking"].in_use, 0);
    assert_eq!(drained.conservation_violation(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn role_pool_saturation_does_not_consume_a_spare_physical_slot() {
    let rt = runtime();
    let (stack_started_tx, stack_started_rx) = tokio::sync::oneshot::channel();
    let (stack_release_tx, stack_release_rx) = std::sync::mpsc::channel::<()>();
    let stack_rt = rt.clone();
    let stack_holder = tokio::spawn(async move {
        stack_rt
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("stack"))
                    .operation("stack-holder")
                    .stack_size_bytes(STACK),
                move || {
                    stack_started_tx.send(()).expect("observer remains live");
                    stack_release_rx.recv().unwrap_or(());
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    tokio::time::timeout(HANG, stack_started_rx)
        .await
        .expect("stack holder starts")
        .expect("stack holder reports start");

    let (plain_started_tx, plain_started_rx) = tokio::sync::oneshot::channel();
    let (plain_release_tx, plain_release_rx) = std::sync::mpsc::channel::<()>();
    let plain_rt = rt.clone();
    let plain_holder = tokio::spawn(async move {
        plain_rt
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("plain")).operation("plain-holder"),
                move || {
                    plain_started_tx.send(()).expect("observer remains live");
                    plain_release_rx.recv().unwrap_or(());
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    tokio::time::timeout(HANG, plain_started_rx)
        .await
        .expect("other physical domain starts")
        .expect("plain holder reports start");

    let held = rt.snapshot();
    assert_eq!(held.capabilities["large_stack"].in_use, 1);
    assert_eq!(held.capabilities["large_stack"].limit, 1);
    assert_eq!(held.capabilities["physical.dedicated"].in_use, 1);
    assert_eq!(held.capabilities["physical.dedicated"].limit, 2);
    assert_eq!(held.capabilities["blocking"].in_use, 1);
    assert_eq!(held.capabilities["physical.shared_blocking"].in_use, 1);

    let follower_started = Arc::new(AtomicUsize::new(0));
    let follower_count = Arc::clone(&follower_started);
    let follower_rt = rt.clone();
    let follower = tokio::spawn(async move {
        follower_rt
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("stack"))
                    .operation("stack-follower")
                    .stack_size_bytes(STACK),
                move || {
                    follower_count.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    tokio::time::timeout(HANG, async {
        loop {
            if rt.snapshot().classes[&TaskClass::new("stack")].queued == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("follower waits on the role pool");
    let queued = rt.snapshot();
    assert_eq!(follower_started.load(Ordering::SeqCst), 0);
    assert_eq!(queued.capabilities["large_stack"].in_use, 1);
    assert_eq!(queued.capabilities["physical.dedicated"].in_use, 1);
    assert_eq!(queued.capabilities["blocking"].in_use, 1);
    assert_eq!(queued.capabilities["physical.shared_blocking"].in_use, 1);
    assert_eq!(queued.conservation_violation(), None);

    stack_release_tx
        .send(())
        .expect("stack holder remains live");
    tokio::time::timeout(HANG, stack_holder)
        .await
        .expect("stack holder completes")
        .expect("stack holder joins")
        .expect("stack holder succeeds");
    tokio::time::timeout(HANG, follower)
        .await
        .expect("follower eventually completes")
        .expect("follower joins")
        .expect("follower succeeds");
    assert_eq!(follower_started.load(Ordering::SeqCst), 1);
    plain_release_tx
        .send(())
        .expect("plain holder remains live");
    tokio::time::timeout(HANG, plain_holder)
        .await
        .expect("plain holder completes")
        .expect("plain holder joins")
        .expect("plain holder succeeds");
    let drained = rt.snapshot();
    assert_eq!(drained.classes[&TaskClass::new("stack")].inflight, 0);
    assert_eq!(drained.classes[&TaskClass::new("stack")].queued, 0);
    assert_eq!(drained.classes[&TaskClass::new("plain")].inflight, 0);
    assert_eq!(drained.capabilities["physical.dedicated"].in_use, 0);
    assert_eq!(drained.capabilities["physical.shared_blocking"].in_use, 0);
    assert_eq!(drained.conservation_violation(), None);
}
