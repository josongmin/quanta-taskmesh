//! D03 on the host: a memory fallback reclassifies the *resource account* of
//! the work, never the policy that governs it.
//!
//! The declared class's cancellation contract, deadline support, and substrate
//! classification stay with the submission; only the ledger the permit is
//! charged to moves to the fallback class. Without this, a class that promised
//! `CooperativeWithDeadline` would silently lose its deadline the moment memory
//! got tight — and the caller would get `Ok` for work that ran unbounded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

const HEAVY: &str = "heavy";
const LIGHT: &str = "light";

/// Memory budget 9: one heavy permit (8) fits, a second does not, a light
/// permit (1) does. The declared policies differ on purpose: heavy can enforce
/// a deadline, light cannot even cancel mid-run.
fn runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(9))
        .class_policy(
            TaskClass::new(HEAVY),
            ClassPolicy::new()
                .max_inflight(8)
                .cpu_units(1)
                .memory_units(8)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline)
                .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
                    fallback_class: TaskClass::new(LIGHT),
                }),
        )
        .class_policy(
            TaskClass::new(LIGHT),
            ClassPolicy::new()
                .max_inflight(8)
                .cpu_units(1)
                .memory_units(1)
                .cancellation_policy(CancellationPolicy::PreSubmitOnly),
        )
        .build()
        .expect("runtime builds")
}

fn heavy(op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new(HEAVY)).operation(op.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_degraded_submission_keeps_its_declared_deadline_contract() {
    let rt = runtime();
    let (release, hold) = tokio::sync::oneshot::channel::<()>();

    // Fill the memory budget with one heavy permit that stays live.
    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_io(heavy("holder"), async move {
                let _released = hold.await;
                Ok::<(), ()>(())
            })
            .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new(HEAVY)].inflight == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "holder never admitted"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    // The second heavy submission overcommits and is re-accounted as light.
    // Its *declared* class supports a deadline, so the deadline is accepted —
    // and enforced: the work is stopped, not run to completion.
    let finished = Arc::new(AtomicBool::new(false));
    let finished_in_work = Arc::clone(&finished);
    let error = rt
        .run_io_with(
            heavy("degraded"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                finished_in_work.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        )
        .await
        .expect_err("the declared class's deadline governs the degraded work");
    assert_eq!(error, RunError::Governor(GovernorError::DeadlineExceeded));
    assert!(!finished.load(Ordering::SeqCst));

    // While it ran, the permit was charged to `light`; the declared class's
    // own account never saw it. Observe that on a live degraded permit.
    let (observe_release, observe_hold) = tokio::sync::oneshot::channel::<()>();
    let rt_degraded = rt.clone();
    let degraded = tokio::spawn(async move {
        rt_degraded
            .run_io(heavy("degraded-live"), async move {
                let _released = observe_hold.await;
                Ok::<(), ()>(())
            })
            .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new(LIGHT)].inflight == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "degraded never admitted"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let snapshot = rt.snapshot();
    assert_eq!(
        snapshot.classes[&TaskClass::new(HEAVY)].inflight,
        1,
        "only the holder"
    );
    assert_eq!(
        snapshot.classes[&TaskClass::new(LIGHT)].inflight,
        1,
        "the degraded one"
    );
    assert_eq!(
        snapshot.classes[&TaskClass::new(LIGHT)].memory_units_held,
        1
    );
    assert_eq!(
        snapshot.classes[&TaskClass::new(HEAVY)].memory_units_held,
        8
    );
    let ledgers = rt.governor().permit_ledgers();
    assert!(
        ledgers
            .iter()
            .any(|l| l.class == TaskClass::new(LIGHT) && l.effective_units == 1),
        "the degraded permit's ledger sits under the fallback class: {ledgers:?}"
    );

    // The fallback class's *own* contract is not what the degraded submission
    // gets: a submission declared as `light` cannot carry a deadline at all.
    let error = rt
        .run_io_with(
            TaskSpec::io(TaskClass::new(LIGHT)).operation("light-with-deadline"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            async { Ok::<(), ()>(()) },
        )
        .await
        .expect_err("light cannot promise a deadline");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::DeadlineUnsupported {
            class: TaskClass::new(LIGHT),
            policy: CancellationPolicy::PreSubmitOnly,
        })
    );

    observe_release.send(()).expect("degraded still held");
    release.send(()).expect("holder still held");
    holder.await.expect("join").expect("holder ok");
    degraded.await.expect("join").expect("degraded ok");
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new(HEAVY)].inflight, 0);
    assert_eq!(snapshot.classes[&TaskClass::new(LIGHT)].inflight, 0);
    assert_eq!(snapshot.conservation_violation(), None);
}
