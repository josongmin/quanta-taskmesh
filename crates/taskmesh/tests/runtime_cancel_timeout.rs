//! T07: cancellation and bounded acquire are adapter-layer concerns, surfaced as
//! typed governor-side rejections.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn single_slot_runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap()
}

fn substrate_contention_runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                // Leave a second class permit available so a blocking waiter
                // queues only on the occupied blocking capability.
                .max_inflight(2)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap()
}

static NEXT_OPERATION: AtomicUsize = AtomicUsize::new(1);

fn spec() -> TaskSpec {
    let id = NEXT_OPERATION.fetch_add(1, Ordering::SeqCst);
    TaskSpec::blocking(TaskClass::new("c")).operation(format!("op-{id}"))
}

async fn wait_until_queue_depth(rt: &TokioRuntime, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if rt.snapshot().classes[&TaskClass::new("c")].queued == expected as u32 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("queue depth never reached {expected}"));
}

fn assert_drained(rt: &TokioRuntime) {
    let snapshot = rt.snapshot();
    let class = &snapshot.classes[&TaskClass::new("c")];
    assert_eq!((class.inflight, class.queued), (0, 0));
    assert_eq!(snapshot.capabilities["blocking"].in_use, 0);
    assert_eq!(snapshot.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 0);
}

async fn assert_spare_class_capacity(rt: &TokioRuntime) {
    // The IO path has different capability requirements from a blocking job.
    // A successful zero-budget execution proves the class itself is not full,
    // independently of the governor's blocker-priority order.
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        rt.run_io_with(
            TaskSpec::io(TaskClass::new("c")).operation("spare-class-probe"),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
            async { Ok::<_, ()>(7) },
        ),
    )
    .await
    .expect("spare-class probe must finish within budget")
    .expect("class capacity remains available while blocking is full");
    assert_eq!(result, 7);
}

#[tokio::test]
async fn cancel_before_submit_is_governor_rejection() {
    let rt = single_slot_runtime();
    let token = CancellationToken::new();
    token.cancel();
    let opts = SubmitOptions::unbounded().with_cancel(token);

    let err = rt
        .run_blocking_with(spec(), opts, || Ok::<i32, ()>(1))
        .await
        .expect_err("cancelled before submit");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
}

#[tokio::test]
async fn cancellation_token_fired_before_submit_rejects() {
    let rt = single_slot_runtime();
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    token.cancel();

    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let err = rt
        .run_io_with(io_spec, opts, async { Ok::<i32, ()>(1) })
        .await
        .expect_err("token fired");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
}

#[tokio::test]
async fn cancel_before_submit_beats_substrate_pool_contention() {
    let rt = single_slot_runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();

    let holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            let mut started_tx = Some(started_tx);
            rt.run_blocking_with(spec(), SubmitOptions::unbounded(), move || {
                started_tx
                    .take()
                    .expect("holder sends started once")
                    .send(())
                    .expect("receiver alive");
                // nosemgrep: taskmesh-test-thread-sleep -- reason: the holder must occupy the slot for longer than the acquire timeout under test; wall-clock is the dimension being tested.
                std::thread::sleep(Duration::from_millis(120));
                Ok::<(), ()>(())
            })
            .await
        }
    });

    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("holder must start within budget")
        .expect("holder start sender survives");

    let token = CancellationToken::new();
    token.cancel();
    let opts = SubmitOptions::unbounded()
        .with_cancel(token)
        .with_acquire_timeout(Duration::from_millis(40));
    let err = rt
        .run_blocking_with(spec(), opts, || Ok::<i32, ()>(1))
        .await
        .expect_err("pre-submit cancellation must win before substrate wait");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));

    tokio::time::timeout(Duration::from_secs(2), holder)
        .await
        .expect("holder must join within budget")
        .expect("holder task must not panic")
        .expect("holder must complete");
    assert_drained(&rt);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_waiting_for_substrate_rejects_without_waiting_for_capacity_v1() {
    let rt = substrate_contention_runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_blocking_with(spec(), SubmitOptions::unbounded(), move || {
                started_tx.send(()).expect("holder start receiver alive");
                // A failed assertion drops the sender and still frees the worker.
                if release_rx.recv().is_err() {
                    return Err(());
                }
                Ok::<(), ()>(())
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("holder must start within budget")
        .expect("holder start sender survives");

    let token = CancellationToken::new();
    let waiter = tokio::spawn({
        let rt = rt.clone();
        let token = token.clone();
        async move {
            rt.run_blocking_with(
                spec(),
                SubmitOptions::unbounded().with_cancel(token),
                || Ok::<(), ()>(()),
            )
            .await
        }
    });

    // The waiter is queued at the governor while class capacity remains: the
    // binding limit is the occupied blocking capability, not the class.
    wait_until_queue_depth(&rt, 1).await;
    let blocked = rt.snapshot();
    assert_eq!(blocked.classes[&TaskClass::new("c")].inflight, 1);
    assert_eq!(blocked.capabilities["blocking"].in_use, 1);
    assert_eq!(blocked.capabilities["blocking"].limit, 1);
    assert_eq!(blocked.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 1);
    assert_spare_class_capacity(&rt).await;
    token.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("pre-admission cancellation must wake substrate wait")
        .expect("submission task must not panic")
        .expect_err("cancelled waiter must reject");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));

    release_tx.send(()).expect("holder must still be running");
    tokio::time::timeout(Duration::from_secs(2), holder)
        .await
        .expect("holder must join within budget")
        .expect("holder task must not panic")
        .expect("holder must complete");
    assert_drained(&rt);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capability_wait_promotes_after_holder_releases() {
    let rt = substrate_contention_runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);

    let holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_blocking(spec(), move || {
                started_tx.send(()).expect("holder start receiver alive");
                // A failed assertion drops the sender and still frees the worker.
                if release_rx.recv().is_err() {
                    return Err(());
                }
                Ok::<_, ()>(1)
            })
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("holder must start within budget")
        .expect("holder start sender survives");

    let waiter = tokio::spawn({
        let rt = rt.clone();
        async move { rt.run_blocking(spec(), || Ok::<_, ()>(2)).await }
    });
    wait_until_queue_depth(&rt, 1).await;
    let blocked = rt.snapshot();
    assert_eq!(blocked.classes[&TaskClass::new("c")].inflight, 1);
    assert_eq!(blocked.capabilities["blocking"].in_use, 1);
    assert_eq!(blocked.capabilities["blocking"].limit, 1);
    assert_eq!(blocked.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 1);
    assert_spare_class_capacity(&rt).await;

    release_tx.send(()).expect("holder remains alive");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), holder)
            .await
            .expect("holder must join within budget")
            .expect("holder task must not panic")
            .expect("holder succeeds"),
        1
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("waiter must promote within budget")
            .expect("waiter task must not panic")
            .expect("waiter succeeds"),
        2
    );
    assert_drained(&rt);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_while_queued_for_governor_abandons_ticket_promptly_v1() {
    let rt = single_slot_runtime();
    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("queued-cancel");
    let occupied = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("c")).operation("queued-cancel-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected direct governor admission, got {other:?}"),
    };

    let token = CancellationToken::new();
    let waiter = tokio::spawn({
        let rt = rt.clone();
        let token = token.clone();
        async move {
            rt.run_io_with(
                io_spec,
                SubmitOptions::unbounded().with_cancel(token),
                async { Ok::<(), ()>(()) },
            )
            .await
        }
    });
    wait_until_queue_depth(&rt, 1).await;

    token.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .expect("pre-admission cancellation must wake governor queue wait")
        .expect("submission task must not panic")
        .expect_err("cancelled waiter must reject");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].queued, 0);

    assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
    assert_drained(&rt);
}

#[tokio::test]
async fn timed_acquire_yields_typed_governor_error() {
    let rt = single_slot_runtime();

    // An IO permit fills the class without charging the blocking capability.
    let occupied = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("c")).operation("class-only-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admit, got {other:?}"),
    };
    let occupied_snapshot = rt.snapshot();
    assert_eq!(occupied_snapshot.classes[&TaskClass::new("c")].inflight, 1);
    assert_eq!(occupied_snapshot.capabilities["blocking"].in_use, 0);
    assert_eq!(occupied_snapshot.capabilities["blocking"].limit, 1);
    assert_eq!(
        occupied_snapshot.capabilities[PHYSICAL_SHARED_BLOCKING].in_use,
        0
    );

    let invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job_invoked = std::sync::Arc::clone(&invoked);
    let err = tokio::time::timeout(
        Duration::from_secs(2),
        rt.run_blocking_with(
            spec(),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
            move || {
                job_invoked.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(1)
            },
        ),
    )
    .await
    .expect("class-only acquisition must end within budget")
    .expect_err("acquire times out");
    assert_eq!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut {
                retry_after_ms: None,
            }
        ))
    );
    assert!(!invoked.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].queued, 0);

    assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
    assert_drained(&rt);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_budget_projects_the_current_capability_blocker_without_running_work() {
    let rt = substrate_contention_runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let holder = {
        let rt = rt.clone();
        tokio::spawn(async move {
            rt.run_blocking(spec(), move || {
                started_tx.send(()).expect("observer alive");
                // A failed assertion drops the sender and still frees the worker.
                if release_rx.recv().is_err() {
                    return Err(());
                }
                Ok::<(), ()>(())
            })
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("holder must start within budget")
        .expect("holder start sender survives");
    let occupied_snapshot = rt.snapshot();
    assert_eq!(occupied_snapshot.classes[&TaskClass::new("c")].inflight, 1);
    assert_eq!(occupied_snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(occupied_snapshot.capabilities["blocking"].limit, 1);
    assert_eq!(
        occupied_snapshot.capabilities[PHYSICAL_SHARED_BLOCKING].in_use,
        1
    );
    assert_spare_class_capacity(&rt).await;

    let invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job_invoked = std::sync::Arc::clone(&invoked);
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        rt.run_blocking_with(
            spec(),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
            move || {
                job_invoked.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect("capability-only acquisition must end within budget")
    .expect_err("try-once sees the current capability blocker");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstratePoolTimedOut {
                retry_after_ms: None,
            }
        ))
    );
    assert!(!invoked.load(Ordering::SeqCst));

    release_tx.send(()).expect("holder alive");
    tokio::time::timeout(Duration::from_secs(2), holder)
        .await
        .expect("holder must join within budget")
        .expect("holder task must not panic")
        .expect("holder succeeds");
    assert_drained(&rt);
}

#[tokio::test]
async fn acquire_timeout_is_one_budget_across_substrate_and_queue_wait() {
    let rt = single_slot_runtime();

    let occupied = match rt.governor().admit(&spec()) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admit, got {other:?}"),
    };

    let queued_holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_blocking_with(
                spec(),
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60)),
                || Ok::<i32, ()>(1),
            )
            .await
        }
    });

    wait_until_queue_depth(&rt, 1).await;

    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60));
    let timed = tokio::time::timeout(
        Duration::from_millis(110),
        rt.run_blocking_with(spec(), opts, || Ok::<i32, ()>(1)),
    )
    .await
    .expect("acquire timeout must be end-to-end, not per-phase")
    .expect_err("acquire must still time out");
    assert!(matches!(
        timed,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
                | AdmissionVerdict::SubstratePoolTimedOut { .. }
        ))
    ));

    let queued_err = tokio::time::timeout(Duration::from_secs(2), queued_holder)
        .await
        .expect("queued holder must join within budget")
        .expect("queued holder task must not panic")
        .expect_err("queued holder must time out");
    assert!(matches!(
        queued_err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
        ))
    ));

    assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
    assert_drained(&rt);
}

#[tokio::test]
async fn acquire_timeout_overflow_fails_closed_before_work_v1() {
    let rt = single_slot_runtime();
    let invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job_invoked = std::sync::Arc::clone(&invoked);

    let error = rt
        .run_blocking_with(
            spec(),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::MAX),
            move || {
                job_invoked.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        )
        .await
        .expect_err("overflowing acquire timeout must fail closed");

    assert_eq!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(
            "acquire timeout exceeds Instant range".into()
        ))
    );
    assert!(!invoked.load(std::sync::atomic::Ordering::SeqCst));
    assert_drained(&rt);
}
