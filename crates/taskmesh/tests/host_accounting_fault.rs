//! A terminal accounting fault wakes a host caller with no acquisition deadline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::ext::ReconcileOutcome;
use taskmesh::*;

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(
            ResourceBudget::new()
                .cpu_units(10)
                .memory_units(1)
                .memory_unit_scale(1),
        )
        .class_policy(
            TaskClass::new("measured"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(1)
                .cpu_units(1)
                .memory_units(1)
                .memory_permit_mode(MemoryPermitMode::Measured)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("measured runtime")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn measured_overflow_terminalizes_unbounded_host_waiter_without_releasing_worker() {
    let rt = runtime();
    let class = TaskClass::new("measured");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let holder_rt = rt.clone();
    let holder = tokio::spawn(async move {
        holder_rt
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("measured")).operation("holder"),
                move || {
                    started_tx.send(()).expect("test waits for worker start");
                    // Dropping release_tx on assertion failure still frees the worker.
                    let _released = release_rx.blocking_recv();
                    Ok::<_, ()>(())
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .expect("worker starts")
        .expect("start signal");
    let ledgers = rt.governor().permit_ledgers();
    assert_eq!(ledgers.len(), 1);
    let holder_permit = ledgers[0].permit_id;
    assert_eq!(ledgers[0].phase, ExecutionPhase::Running);

    let ran = Arc::new(AtomicBool::new(false));
    let waiter_ran = Arc::clone(&ran);
    let waiter_rt = rt.clone();
    let waiter = tokio::spawn(async move {
        waiter_rt
            .run_io(
                TaskSpec::io(TaskClass::new("measured")).operation("waiter"),
                async move {
                    waiter_ran.store(true, Ordering::SeqCst);
                    Ok::<_, ()>(())
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if rt.snapshot().classes[&class].queued == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deadline-free caller queued");

    assert!(matches!(
        rt.governor()
            .reconcile_memory_at(holder_permit, u64::from(u32::MAX) + 1, 1),
        ReconcileOutcome::ConversionFailed(_)
    ));
    let error = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("terminal wake bounds the wait")
        .expect("waiter task joins")
        .expect_err("accounting fault terminates the ticket");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::TicketClaimTerminated {
            reason: TerminalReason::AccountingFault,
            ..
        })
    ));
    assert!(!ran.load(Ordering::SeqCst), "terminal waiter never ran");
    let snapshot = rt.snapshot();
    assert_eq!(
        (
            snapshot.classes[&class].inflight,
            snapshot.classes[&class].queued
        ),
        (1, 0)
    );
    assert_eq!(
        rt.governor()
            .permit_ledger(holder_permit)
            .expect("worker still charged")
            .phase,
        ExecutionPhase::Running
    );
    let not_drained = rt
        .drain(Duration::ZERO)
        .await
        .expect_err("drain cannot finish with live worker");
    assert_eq!(
        not_drained.classes.get(&class),
        Some(&Outstanding {
            inflight: 1,
            queued: 0,
        })
    );
    assert!(!holder.is_finished(), "measured worker still holds custody");

    release_tx.send(()).expect("worker still waits for release");
    tokio::time::timeout(Duration::from_secs(5), holder)
        .await
        .expect("holder finishes")
        .expect("holder task joins")
        .expect("holder work succeeds");
    rt.drain(Duration::from_secs(5))
        .await
        .expect("settled runtime drains");
    let snapshot = rt.snapshot();
    assert_eq!(
        (
            snapshot.classes[&class].inflight,
            snapshot.classes[&class].queued
        ),
        (0, 0)
    );
    assert_eq!(snapshot.conservation_violation(), None);
}
