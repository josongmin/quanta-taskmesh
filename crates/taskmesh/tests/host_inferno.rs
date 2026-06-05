//! Inferno · host (T07/T09): the real `TokioRuntime` execution paths under
//! abuse. Panicking work, substrate-hint mismatch, cooperative vs pre-submit
//! cancellation, queue-full, cancellation-safety of queued submissions, the
//! topology substrate gate, and concurrent promotion — every one asserted to a
//! typed outcome with no permit leaked.

use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;
use tokio::time::{sleep, timeout};

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
            res,
            Err(RunError::Governor(GovernorError::PolicyViolation(_)))
        ),
        "a panicking blocking task is a join failure (governor-side), not a task error"
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
            res,
            Err(RunError::Governor(GovernorError::PolicyViolation(_)))
        ),
        "a panicking cpu task must surface as a governor-side error"
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

// ---- substrate classification is authoritative -----------------------------

#[tokio::test]
async fn substrate_hint_mismatch_is_fail_closed() {
    let rt = rt_default();
    let io_on_blocking: Result<i32, RunError<()>> =
        rt.run_io(blk("c", "x"), async { Ok::<i32, ()>(1) }).await;
    assert!(matches!(
        io_on_blocking,
        Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstrateMismatch
        )))
    ));

    let cpu_on_io: Result<i32, RunError<()>> = rt
        .run_cpu(TaskSpec::io(TaskClass::new("c")).operation("x"), || Ok(1))
        .await;
    assert!(matches!(
        cpu_on_io,
        Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstrateMismatch
        )))
    ));

    let blocking_on_cpu: Result<i32, RunError<()>> = rt
        .run_blocking(TaskSpec::cpu(TaskClass::new("c")).operation("x"), || Ok(1))
        .await;
    assert!(matches!(
        blocking_on_cpu,
        Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstrateMismatch
        )))
    ));

    let local_on_blocking: Result<i32, RunError<()>> = rt
        .run_local(blk("c", "x"), async { Ok::<i32, ()>(1) })
        .await;
    assert!(
        matches!(
            local_on_blocking,
            Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::SubstrateMismatch
            )))
        ),
        "run_local is the local-runtime exception, not a general path"
    );
}

// ---- cancellation policy gate ----------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cooperative_class_cancels_mid_run() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("coop"),
            ClassPolicy::new()
                .max_inflight(4)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::Cooperative),
        )
        .build()
        .unwrap();

    let token = CancellationToken::new();
    let (rt2, tk) = (rt.clone(), token.clone());
    let h = tokio::spawn(async move {
        rt2.run_io_with(
            TaskSpec::io(TaskClass::new("coop")).operation("long"),
            SubmitOptions::unbounded().with_cancel(tk),
            async {
                sleep(Duration::from_secs(10)).await;
                Ok::<i32, ()>(1)
            },
        )
        .await
    });
    sleep(Duration::from_millis(30)).await;
    token.cancel();

    let err = h.await.unwrap().expect_err("cooperative cancel mid-run");
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));
    assert_eq!(
        inflight(&rt, "coop"),
        0,
        "cancelled run must release its permit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_submit_only_class_ignores_mid_run_cancel() {
    // Default policy is PreSubmitOnly: a token fired *after* admission is ignored.
    let rt = rt_default();
    let token = CancellationToken::new();
    let (rt2, tk) = (rt.clone(), token.clone());
    let h = tokio::spawn(async move {
        rt2.run_io_with(
            TaskSpec::io(TaskClass::new("c")).operation("x"),
            SubmitOptions::unbounded().with_cancel(tk),
            async {
                sleep(Duration::from_millis(150)).await;
                Ok::<i32, ()>(99)
            },
        )
        .await
    });
    sleep(Duration::from_millis(20)).await;
    token.cancel(); // mid-run — must be ignored by a PreSubmitOnly class

    let out = h
        .await
        .unwrap()
        .expect("PreSubmitOnly ignores mid-run cancel");
    assert_eq!(out, 99);
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
    g.release(occupy);
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
    g.release(occupy);
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
async fn substrate_gate_caps_blocking_concurrency_and_times_out() {
    // One blocking slot in the topology. A holder occupies it; a second
    // submission with a bounded acquire wait must time out on the gate.
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rt2 = rt.clone();
    let holder = tokio::spawn(async move {
        rt2.run_blocking(blk("c", "hold"), move || {
            let _ = rx.recv(); // hold the single blocking gate until signalled
            Ok::<i32, ()>(0)
        })
        .await
    });
    sleep(Duration::from_millis(50)).await; // let the holder seize the gate

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
        "a full substrate gate must yield a bounded-acquire timeout"
    );

    let _ = tx.send(()); // release the holder
    holder.await.unwrap().unwrap();
    assert_eq!(
        inflight(&rt, "c"),
        0,
        "no permit leaked through the gated path"
    );
}
