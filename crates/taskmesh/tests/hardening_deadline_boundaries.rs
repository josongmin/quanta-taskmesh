//! D05: an unrepresentable RunFor bound has different async and detached-sync
//! contracts. Async work fails before polling; an already-started synchronous
//! worker retains its lease while its unrepresentable timer is unbounded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::*;

const HANG: Duration = Duration::from_secs(5);
const STACK: u64 = 2 * 1024 * 1024;

fn class() -> TaskClass {
    TaskClass::new("c")
}

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(1)
                .large_stack_slots(1),
        )
        .resources(ResourceBudget::new().cpu_units(2).memory_units(2))
        .class_policy(
            class(),
            ClassPolicy::new()
                .max_inflight(1)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .expect("bounded deadline runtime")
}

fn assert_unrepresentable_from_now() {
    assert!(
        Instant::now().checked_add(Duration::MAX).is_none(),
        "this fixture requires an unrepresentable RunFor bound"
    );
}

fn assert_policy_overflow<E: std::fmt::Debug>(result: Result<(), RunError<E>>) {
    assert!(
        matches!(
            result,
            Err(RunError::Governor(GovernorError::PolicyViolation(message)))
                if message == "relative run deadline exceeds Instant range"
        ),
        "overflow must be a typed PolicyViolation before polling"
    );
}

fn assert_idle(rt: &TokioRuntime) {
    let snapshot = rt.snapshot();
    let state = &snapshot.classes[&class()];
    assert_eq!((state.inflight, state.queued), (0, 0));
    assert!(
        snapshot.capabilities.values().all(|pool| pool.in_use == 0),
        "all role and physical pools must be refunded"
    );
    assert_eq!(snapshot.conservation_violation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn async_io_local_and_requested_stack_runfor_overflow_refund_without_polling() {
    assert_unrepresentable_from_now();
    let rt = runtime();
    let opts = || SubmitOptions::unbounded().with_deadline(Duration::MAX);

    let io_polled = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&io_polled);
    let result = tokio::time::timeout(
        HANG,
        rt.run_io_with(
            TaskSpec::io(class()).operation("io-overflow"),
            opts(),
            async move {
                seen.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect("IO overflow returns");
    assert_policy_overflow(result);
    assert!(!io_polled.load(Ordering::SeqCst));
    assert_idle(&rt);

    let local_polled = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&local_polled);
    let result = tokio::time::timeout(
        HANG,
        rt.run_local_with(
            TaskSpec::local(class()).operation("local-overflow"),
            opts(),
            async move {
                let _local_only = std::rc::Rc::new(());
                seen.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect("local overflow returns");
    assert_policy_overflow(result);
    assert!(!local_polled.load(Ordering::SeqCst));
    assert_idle(&rt);

    let stack_polled = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&stack_polled);
    let result = tokio::time::timeout(
        HANG,
        rt.run_async_with_requested_stack_with(
            TaskSpec::base(class(), SubstrateHint::LargeStackCapability)
                .operation("stack-overflow")
                .stack_size_bytes(STACK),
            opts(),
            move || async move {
                seen.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect("requested-stack overflow returns");
    assert_policy_overflow(result);
    assert!(!stack_polled.load(Ordering::SeqCst));
    assert_idle(&rt);

    let control: () = rt
        .run_io(
            TaskSpec::io(class()).operation("capacity-refunded"),
            async { Ok::<(), ()>(()) },
        )
        .await
        .expect("overflow returned all admission capacity");
    assert_eq!(control, ());
    assert_idle(&rt);
}

struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl ReleaseOnDrop {
    fn release(mut self) {
        self.0
            .take()
            .expect("one release")
            .send(())
            .expect("worker held");
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn started_sync_runfor_overflow_keeps_worker_custody_until_completion() {
    assert_unrepresentable_from_now();
    let rt = runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release_tx));
    let completed = Arc::new(AtomicBool::new(false));
    let completed_by_worker = Arc::clone(&completed);
    let worker_rt = rt.clone();

    let mut caller = tokio::spawn(async move {
        worker_rt
            .run_blocking_with(
                TaskSpec::blocking(class()).operation("sync-overflow"),
                SubmitOptions::unbounded().with_deadline(Duration::MAX),
                move || {
                    let _observer_gone = started_tx.send(());
                    release_rx.recv().expect("test releases worker");
                    completed_by_worker.store(true, Ordering::SeqCst);
                    Ok::<i32, ()>(7)
                },
            )
            .await
    });
    tokio::time::timeout(HANG, started_rx)
        .await
        .expect("worker started")
        .expect("worker announced start");
    assert_eq!(rt.snapshot().classes[&class()].inflight, 1);
    tokio::time::timeout(Duration::from_millis(30), &mut caller)
        .await
        .expect_err("an unrepresentable RunFor must not answer while the worker is held");
    assert!(!completed.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&class()].inflight, 1);

    let rejected_work_ran = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&rejected_work_ran);
    let second = tokio::time::timeout(
        HANG,
        rt.run_blocking_with(
            TaskSpec::blocking(class()).operation("must-not-resell"),
            SubmitOptions::unbounded(),
            move || {
                seen.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(9)
            },
        ),
    )
    .await
    .expect("second admission is bounded");
    assert!(
        matches!(&second, Err(RunError::Governor(GovernorError::Rejected(_)))),
        "a live worker retains the only class slot: {second:?}"
    );
    assert!(!rejected_work_ran.load(Ordering::SeqCst));
    let not_drained = rt
        .drain(Duration::ZERO)
        .await
        .expect_err("drain cannot succeed before the held worker terminates");
    assert_eq!(not_drained.classes[&class()].inflight, 1);

    release.release();
    let result = tokio::time::timeout(HANG, caller)
        .await
        .expect("caller answers after worker release")
        .expect("caller task survives")
        .expect("unbounded detached timer does not fail the result");
    assert_eq!(result, 7);
    assert!(completed.load(Ordering::SeqCst));
    assert_idle(&rt);
    rt.drain(Duration::from_secs(1))
        .await
        .expect("worker custody settled before drain");
}
