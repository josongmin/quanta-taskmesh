//! H28: compare real host and discrete-event simulator on the same finite
//! open-loop overload. Only semantic counts and queue bounds are comparable:
//! simulator latency is virtual admission wait, not host response latency.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::{
    AdmissionVerdict, Builder, ClassPolicy, GovernorError, OverflowPolicy, ResourceBudget,
    RunError, Runtime, TaskClass, TaskSpec,
};
use taskmesh_bench::loadgen::simulate;
use taskmesh_bench::workload::{fixture, Arrival, ValidatedArrivals};

const HANG: Duration = Duration::from_secs(5);
const FOLLOWERS: usize = 8;

fn policy() -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(2)
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
}

fn spec(operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(operation.to_owned())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_host_and_simulator_agree_on_bounded_burst_accounting() {
    // One admitted holder plus eight arrivals before that holder can finish.
    // Both engines see the same class policy and budget. A fixed virtual
    // service interval and a gated real worker express the same overload
    // ordering without pretending their wall-clock latencies are comparable.
    let arrivals = ValidatedArrivals::new(
        (0..=FOLLOWERS)
            .map(|_| Arrival {
                send_time_secs: 0.0,
                class: "c".to_owned(),
            })
            .collect(),
    )
    .expect("finite simultaneous schedule");
    let simulation = fixture(vec![("c", policy())], 100, 100);
    let (virtual_wait, simulated) =
        simulate(&simulation, &arrivals, 1_000_000).expect("finite simulation");
    assert!(simulated.is_conserved());
    assert_eq!(simulated.offered, 1 + FOLLOWERS);
    assert_eq!(simulated.max_queue_observed, 2);
    assert_eq!(simulated.leftover_queued, 0);
    assert_eq!(virtual_wait.len() as usize, simulated.started());
    assert_eq!(virtual_wait.synthetic_samples(), 0);

    let runtime = Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(TaskClass::new("c"), policy())
        .build()
        .expect("host runtime builds");
    let executed = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = {
        let runtime = runtime.clone();
        let executed = Arc::clone(&executed);
        tokio::spawn(async move {
            runtime
                .run_blocking(
                    TaskSpec::blocking(TaskClass::new("c")).operation("holder"),
                    move || {
                        executed.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).expect("holder observer alive");
                        // A dropped sender also releases this worker if an
                        // assertion fails before the explicit release.
                        let _ = release_rx.recv();
                        Ok::<_, ()>(())
                    },
                )
                .await
        })
    };
    tokio::time::timeout(HANG, started_rx)
        .await
        .expect("real holder starts")
        .expect("holder signals start");

    let mut followers = Vec::new();
    for index in 0..FOLLOWERS {
        let runtime = runtime.clone();
        let executed = Arc::clone(&executed);
        followers.push(tokio::spawn(async move {
            runtime
                .run_io(spec(&format!("follower-{index}")), async move {
                    executed.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ()>(())
                })
                .await
        }));
    }
    tokio::time::timeout(HANG, async {
        loop {
            let class = &runtime.snapshot().classes[&TaskClass::new("c")];
            if class.queued == 2 && followers.iter().filter(|task| task.is_finished()).count() == 6
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("host reaches full queue and six terminal rejects");
    let host_peak_queue = runtime.snapshot().classes[&TaskClass::new("c")].queued;
    release_tx.send(()).expect("holder remains live");

    tokio::time::timeout(HANG, holder)
        .await
        .expect("holder terminates")
        .expect("holder joins")
        .expect("holder succeeds");
    let mut host_completed = 1_usize;
    let mut host_rejected = 0_usize;
    for follower in followers {
        match tokio::time::timeout(HANG, follower)
            .await
            .expect("follower responds")
            .expect("follower joins")
        {
            Ok(()) => host_completed += 1,
            Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::QueueFull {
                ..
            }))) => host_rejected += 1,
            other => panic!("unexpected host terminal: {other:?}"),
        }
    }
    let host_offered = 1 + FOLLOWERS;
    let host_terminal = host_completed + host_rejected;
    let host_unanswered = host_offered - host_terminal;
    assert_eq!(host_unanswered, 0);
    assert_eq!(executed.load(Ordering::SeqCst), host_completed);
    assert_eq!(runtime.snapshot().classes[&TaskClass::new("c")].inflight, 0);
    assert_eq!(runtime.snapshot().classes[&TaskClass::new("c")].queued, 0);

    assert_eq!(host_offered, simulated.offered);
    assert_eq!(host_completed, simulated.completed);
    assert_eq!(host_rejected, simulated.rejected);
    assert_eq!(
        usize::try_from(host_peak_queue).unwrap(),
        simulated.max_queue_observed
    );
}
