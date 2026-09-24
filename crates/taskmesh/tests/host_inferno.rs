//! Inferno · host (T07/T09): the real `TokioRuntime` execution paths under
//! abuse. Panicking work, queue-full, cancellation-safety of queued submissions,
//! the topology substrate gate, and concurrent promotion — every one asserted
//! to a typed outcome with no permit leaked. Mid-run cancellation has a single
//! stronger owner in `cancellation_policy.rs`.

use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;
use tokio::time::timeout;

fn blk(class: &str, op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string())
}

fn rt_default() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

fn single_slot(inflight: u32, depth: u32) -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(inflight)
                .max_queue_depth(depth)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap()
}

fn inflight(rt: &TokioRuntime, class: &str) -> u32 {
    rt.snapshot().classes[&TaskClass::new(class.to_string())].inflight
}
fn queued(rt: &TokioRuntime, class: &str) -> u32 {
    rt.snapshot().classes[&TaskClass::new(class.to_string())].queued
}

// ---- panicking work is a governor/runtime error, never a task error, and
//      never leaks the permit -------------------------------------------------

#[tokio::test]
async fn panicking_blocking_task_is_governor_error_and_releases_permit() {
    let rt = rt_default();
    let res: Result<i32, RunError<()>> = rt.run_blocking(blk("c", "boom"), || panic!("boom")).await;
    assert!(
        matches!(
            &res,
            Err(RunError::Governor(GovernorError::WorkerPanicked { context }))
                if context == "blocking worker"
        ),
        "a panicking blocking task is a typed worker panic (governor-side), not a task error: {res:?}"
    );
    assert_eq!(inflight(&rt, "c"), 0, "panic must not leak the permit");
    // The runtime is still fully usable afterwards.
    let ok: i32 = rt
        .run_blocking(blk("c", "ok"), || Ok::<i32, ()>(7))
        .await
        .unwrap();
    assert_eq!(ok, 7);
}

#[tokio::test]
async fn panicking_cpu_task_is_governor_error_and_releases_permit() {
    let rt = rt_default();
    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("boom");
    let res: Result<i32, RunError<()>> = rt.run_cpu(spec, || panic!("boom")).await;
    assert!(
        matches!(
            &res,
            Err(RunError::Governor(GovernorError::WorkerPanicked { context }))
                if context == "cpu worker"
        ),
        "a panicking cpu task must surface as a typed worker panic: {res:?}"
    );
    assert_eq!(inflight(&rt, "c"), 0, "panic must not leak the permit");
}

#[tokio::test]
async fn task_error_stays_a_task_error() {
    let rt = rt_default();
    let res: Result<i32, RunError<&str>> = rt
        .run_blocking(blk("c", "e"), || Err::<i32, &str>("domain failure"))
        .await;
    assert!(matches!(res, Err(RunError::Task("domain failure"))));
    assert_eq!(inflight(&rt, "c"), 0);
}

// ---- bounded queue + cancellation-safety -----------------------------------

#[tokio::test]
async fn queue_full_is_a_typed_rejection() {
    let rt = single_slot(1, 2);
    let g = rt.governor();
    let occupy = match g.admit(&blk("c", "o")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("occupy must admit, got {o:?}"),
    };
    // Fill the queue to its depth of 2.
    for op in ["q1", "q2"] {
        assert!(matches!(
            g.admit(&blk("c", op)),
            AdmissionDecision::Queued { .. }
        ));
    }
    // The host submission past depth is rejected QueueFull.
    let r: Result<i32, RunError<()>> = rt.run_blocking(blk("c", "q3"), || Ok(1)).await;
    assert!(matches!(
        r,
        Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::QueueFull { .. }
        )))
    ));
    assert_eq!(g.release(occupy), ReleaseOutcome::Released);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_queued_submission_abandons_it_without_leak() {
    let rt = single_slot(1, 8);
    let g = rt.governor();
    let occupy = match g.admit(&blk("c", "o")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    // This submission must queue behind the occupied slot; we then drop it.
    let fut = rt.run_blocking(blk("c", "q"), || Ok::<i32, ()>(1));
    // nosemgrep: taskmesh-test-is-err-without-error-check -- timeout has one error outcome; cleanup state is asserted below
    assert!(
        timeout(Duration::from_millis(60), fut).await.is_err(),
        "submission stays queued behind the held slot"
    );
    // The dropped future's TicketGuard abandoned the ticket → queue is empty.
    assert_eq!(
        queued(&rt, "c"),
        0,
        "a cancelled queued request must not leak"
    );
    assert_eq!(g.release(occupy), ReleaseOutcome::Released);
    assert_eq!(inflight(&rt, "c"), 0);
}

// ---- real-async promotion + concurrency ------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_submissions_promote_and_all_complete() {
    let rt = single_slot(2, 1_000);
    let mut handles = Vec::new();
    for i in 0..64usize {
        let rt = rt.clone();
        handles.push(tokio::spawn(async move {
            rt.run_blocking(blk("c", &format!("j{i}")), move || Ok::<usize, ()>(i))
                .await
        }));
    }
    let mut sum = 0usize;
    for h in handles {
        sum += h.await.unwrap().unwrap();
    }
    assert_eq!(
        sum,
        (0..64).sum::<usize>(),
        "every queued submission completes exactly once"
    );
    assert_eq!(inflight(&rt, "c"), 0, "all permits released");
    assert_eq!(queued(&rt, "c"), 0, "queue fully drained");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queueing_class_waits_for_a_capability_slot_then_reports_the_pool_timeout() {
    // The queueing counterpart: the same saturated pool, but a class that opted
    // into waiting. It waits inside the *bounded* class queue, and the timeout
    // names the capability pool rather than the generic admission wait, so the
    // two backpressure causes stay distinguishable to an operator.
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(4)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let rt2 = rt.clone();
    let holder = tokio::spawn(async move {
        rt2.run_blocking(blk("c", "hold"), move || {
            started_tx.send(()).expect("test observes holder start");
            // A signal or a dropped sender both release the holder: a test that fails
            // before signalling never hangs on its own fixture.
            let _released = rx.recv();
            Ok::<i32, ()>(0)
        })
        .await
    });
    timeout(Duration::from_secs(1), started_rx)
        .await
        .expect("holder starts within the test bound")
        .expect("holder start sender survives");

    let r: Result<i32, RunError<()>> = rt
        .run_blocking_with(
            blk("c", "blocked"),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(50)),
            || Ok(1),
        )
        .await;
    assert!(
        matches!(
            r,
            Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::SubstratePoolTimedOut { .. }
            )))
        ),
        "a bounded wait on a full capability pool reports the pool, got {r:?}"
    );
    assert_eq!(queued(&rt, "c"), 0, "the timed-out ticket left the queue");

    tx.send(())
        .expect("the holder is still waiting for its release");
    holder.await.unwrap().unwrap();
    assert_eq!(inflight(&rt, "c"), 0, "no permit leaked");
}
