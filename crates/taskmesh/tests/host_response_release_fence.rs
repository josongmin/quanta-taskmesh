//! A promoted waiter's synchronous wake is part of the response release fence.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use taskmesh::ext::PermitWaker;
use taskmesh::*;

const STACK: u64 = 2 * 1024 * 1024;
// The setup horizon is shared by late and on-time controls. The late oracle
// is the observed callback crossing this boundary, not a relative sleep ratio.
const RESPONSE_HORIZON: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
enum Path {
    Io,
    Blocking,
    DedicatedBlocking,
    Cpu,
    RequestedStackAsync,
}

struct SlowWake {
    delay: Duration,
    hold_until: Option<Instant>,
    started: Mutex<Option<Instant>>,
    finished: Mutex<Option<Instant>>,
    finish_signal: tokio::sync::Notify,
    cancel_after: Option<CancellationToken>,
}

impl SlowWake {
    fn new(
        delay: Duration,
        hold_until: Option<Instant>,
        cancel_after: Option<CancellationToken>,
    ) -> Self {
        Self {
            delay,
            hold_until,
            started: Mutex::new(None),
            finished: Mutex::new(None),
            finish_signal: tokio::sync::Notify::new(),
            cancel_after,
        }
    }

    async fn wait_finished(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.finish_signal.notified())
            .await
            .expect("release callback finishes");
    }

    fn assert_crossed(&self, response_by: Instant) {
        let started = self
            .started
            .lock()
            .expect("wake timestamp lock")
            .expect("wake called");
        let finished = self
            .finished
            .lock()
            .expect("wake timestamp lock")
            .expect("wake finished");
        assert!(started < response_by, "callback begins before the boundary");
        assert!(finished >= response_by, "callback crosses the boundary");
    }
}

impl PermitWaker for SlowWake {
    fn wake(&self) {
        *self.started.lock().expect("wake timestamp lock") = Some(Instant::now());
        if let Some(boundary) = self.hold_until {
            loop {
                let remaining = boundary.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                std::thread::sleep(remaining);
            }
        } else {
            std::thread::sleep(self.delay);
        }
        if let Some(token) = &self.cancel_after {
            token.cancel();
        }
        *self.finished.lock().expect("wake timestamp lock") = Some(Instant::now());
        self.finish_signal.notify_one();
    }
}

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(1)
                .large_stack_slots(1)
                .cpu_fixed(1),
        )
        .resources(ResourceBudget::new().cpu_units(10).memory_units(10))
        .class_policy(
            TaskClass::new("release-fence"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(1)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .expect("release-fence runtime")
}

async fn exercise(path: Path, expect_deadline: bool) {
    let rt = runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Instant>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let submitted_rt = rt.clone();
    let submission = tokio::spawn(async move {
        // Spawn queue time is not part of this fixture's release-fence oracle.
        // Absolute caller deadlines still cover acquisition/worker dispatch.
        let response_by = Instant::now() + RESPONSE_HORIZON;
        let class = TaskClass::new("release-fence");
        match path {
            Path::Io => {
                submitted_rt
                    .run_io_with(
                        TaskSpec::io(class).operation("running"),
                        SubmitOptions::unbounded().with_absolute_deadline(response_by),
                        async move {
                            started_tx.send(response_by).expect("test waits for start");
                            release_rx.await.expect("test releases work");
                            Ok::<_, ()>(7)
                        },
                    )
                    .await
            }
            Path::Blocking | Path::DedicatedBlocking => {
                let mut spec = TaskSpec::blocking(class).operation("running");
                if matches!(path, Path::DedicatedBlocking) {
                    spec = spec.stack_size_bytes(STACK);
                }
                submitted_rt
                    .run_blocking_response_by(
                        spec,
                        SubmitOptions::unbounded(),
                        response_by,
                        move || {
                            started_tx.send(response_by).expect("test waits for start");
                            release_rx.blocking_recv().expect("test releases work");
                            Ok::<_, ()>(7)
                        },
                    )
                    .await
            }
            Path::Cpu => {
                submitted_rt
                    .run_cpu_response_by(
                        TaskSpec::cpu(class).operation("running"),
                        SubmitOptions::unbounded(),
                        response_by,
                        move || {
                            started_tx.send(response_by).expect("test waits for start");
                            release_rx.blocking_recv().expect("test releases work");
                            Ok::<_, ()>(7)
                        },
                    )
                    .await
            }
            Path::RequestedStackAsync => {
                submitted_rt
                    .run_async_with_requested_stack_with(
                        TaskSpec::base(class, SubstrateHint::LargeStackCapability)
                            .operation("running")
                            .stack_size_bytes(STACK),
                        SubmitOptions::unbounded().with_absolute_deadline(response_by),
                        move || async move {
                            started_tx.send(response_by).expect("test waits for start");
                            release_rx.await.expect("test releases work");
                            Ok::<_, ()>(7)
                        },
                    )
                    .await
            }
        }
    });
    let response_by = match tokio::time::timeout(Duration::from_secs(5), started_rx).await {
        Ok(Ok(boundary)) => boundary,
        start => {
            let caller = tokio::time::timeout(Duration::from_secs(5), submission).await;
            panic!("{path:?}: worker-start precondition failed: {start:?}; caller={caller:?}");
        }
    };
    let wake = Arc::new(SlowWake::new(
        Duration::ZERO,
        expect_deadline.then_some(response_by),
        None,
    ));
    let waiting = TaskSpec::io(TaskClass::new("release-fence")).operation("waiting");
    let ticket = match rt.governor().admit_waitable(&waiting, wake.clone()) {
        taskmesh::ext::AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queued request, got {other:?}"),
    };
    release_tx.send(()).expect("work still waits for release");
    let result = tokio::time::timeout(Duration::from_secs(5), submission)
        .await
        .expect("caller response")
        .expect("submission task");
    // A terminal caller timer may win before a detached callback returns.
    // Observe callback completion separately before claiming fence evidence.
    wake.wait_finished().await;
    if expect_deadline {
        assert!(
            matches!(
                result,
                Err(RunError::Governor(GovernorError::DeadlineExceeded))
            ),
            "{path:?}: {result:?}"
        );
        wake.assert_crossed(response_by);
    } else {
        assert_eq!(result.expect("on-time response"), 7);
    }
    let permit = match rt.governor().claim(ticket) {
        taskmesh::ext::ClaimOutcome::Ready(permit) => permit,
        other => panic!("promoted waiter not ready: {other:?}"),
    };
    assert!(matches!(
        rt.governor().release(permit),
        taskmesh::ext::ReleaseOutcome::Released
    ));
    let snapshot = rt.snapshot();
    let class = &snapshot.classes[&TaskClass::new("release-fence")];
    assert_eq!((class.inflight, class.queued), (0, 0));
    assert_eq!(snapshot.conservation_violation(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn release_callback_cannot_turn_late_responses_into_success() {
    for path in [
        Path::Io,
        Path::Blocking,
        Path::DedicatedBlocking,
        Path::Cpu,
        Path::RequestedStackAsync,
    ] {
        exercise(path, true).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_time_release_still_returns_the_value() {
    for path in [
        Path::Io,
        Path::Blocking,
        Path::DedicatedBlocking,
        Path::Cpu,
        Path::RequestedStackAsync,
    ] {
        exercise(path, false).await;
    }
}

async fn exercise_local(expect_deadline: bool) {
    let rt = runtime();
    let response_by = Instant::now() + RESPONSE_HORIZON;
    let wake = Arc::new(SlowWake::new(
        Duration::ZERO,
        expect_deadline.then_some(response_by),
        None,
    ));
    let ticket = Arc::new(Mutex::new(None));
    let work_rt = rt.clone();
    let work_wake = Arc::clone(&wake);
    let work_ticket = Arc::clone(&ticket);
    let result = rt
        .run_local_with(
            TaskSpec::local(TaskClass::new("release-fence")).operation("running"),
            SubmitOptions::unbounded().with_absolute_deadline(response_by),
            async move {
                let local_only = std::rc::Rc::new(7);
                tokio::task::yield_now().await;
                assert_eq!(*local_only, 7);
                let waiting = TaskSpec::io(TaskClass::new("release-fence")).operation("waiting");
                let queued = match work_rt.governor().admit_waitable(&waiting, work_wake) {
                    taskmesh::ext::AdmissionDecision::Queued { ticket } => ticket,
                    other => panic!("expected queued request, got {other:?}"),
                };
                *work_ticket.lock().expect("ticket lock") = Some(queued);
                Ok::<_, ()>(7)
            },
        )
        .await;
    wake.wait_finished().await;
    if expect_deadline {
        assert!(matches!(
            result,
            Err(RunError::Governor(GovernorError::DeadlineExceeded))
        ));
        wake.assert_crossed(response_by);
    } else {
        assert_eq!(result.expect("on-time local response"), 7);
    }
    let queued = ticket
        .lock()
        .expect("ticket lock")
        .take()
        .expect("ticket set");
    let permit = match rt.governor().claim(queued) {
        taskmesh::ext::ClaimOutcome::Ready(permit) => permit,
        other => panic!("promoted local waiter not ready: {other:?}"),
    };
    assert!(matches!(
        rt.governor().release(permit),
        taskmesh::ext::ReleaseOutcome::Released
    ));
    let snapshot = rt.snapshot();
    assert_eq!(
        snapshot.classes[&TaskClass::new("release-fence")].inflight,
        0
    );
    assert_eq!(snapshot.conservation_violation(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn local_release_callback_obeys_response_deadline_and_on_time_control() {
    exercise_local(true).await;
    exercise_local(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_for_counts_worker_time_without_the_release_callback() {
    let rt = runtime();
    let wake = Arc::new(SlowWake::new(Duration::from_millis(200), None, None));
    let ticket = Arc::new(Mutex::new(None));
    let work_rt = rt.clone();
    let work_wake = Arc::clone(&wake);
    let work_ticket = Arc::clone(&ticket);
    let result = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("release-fence")).operation("running"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(100)),
            move || {
                let waiting = TaskSpec::io(TaskClass::new("release-fence")).operation("waiting");
                let queued = match work_rt.governor().admit_waitable(&waiting, work_wake) {
                    taskmesh::ext::AdmissionDecision::Queued { ticket } => ticket,
                    other => panic!("expected queued request, got {other:?}"),
                };
                *work_ticket.lock().expect("ticket lock") = Some(queued);
                Ok::<_, ()>(7)
            },
        )
        .await;
    assert_eq!(result.expect("worker completed inside RunFor"), 7);
    assert!(wake.started.lock().expect("wake timestamp lock").is_some());
    let queued = ticket
        .lock()
        .expect("ticket lock")
        .take()
        .expect("ticket set");
    let permit = match rt.governor().claim(queued) {
        taskmesh::ext::ClaimOutcome::Ready(permit) => permit,
        other => panic!("promoted waiter not ready: {other:?}"),
    };
    assert!(matches!(
        rt.governor().release(permit),
        taskmesh::ext::ReleaseOutcome::Released
    ));
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("release-fence")].inflight,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_task_error_becomes_deadline_and_cancel_wins_the_tie() {
    for cancel_during_release in [false, true] {
        let rt = runtime();
        let token = CancellationToken::new();
        let response_by = Instant::now() + RESPONSE_HORIZON;
        let wake = Arc::new(SlowWake::new(
            Duration::ZERO,
            Some(response_by),
            cancel_during_release.then(|| token.clone()),
        ));
        let ticket = Arc::new(Mutex::new(None));
        let work_rt = rt.clone();
        let work_wake = Arc::clone(&wake);
        let work_ticket = Arc::clone(&ticket);
        let mut options = SubmitOptions::unbounded().with_absolute_deadline(response_by);
        if cancel_during_release {
            options = options.with_cancel(token);
        }
        let result = rt
            .run_io_with(
                TaskSpec::io(TaskClass::new("release-fence")).operation("running"),
                options,
                async move {
                    let waiting =
                        TaskSpec::io(TaskClass::new("release-fence")).operation("waiting");
                    let queued = match work_rt.governor().admit_waitable(&waiting, work_wake) {
                        taskmesh::ext::AdmissionDecision::Queued { ticket } => ticket,
                        other => panic!("expected queued request, got {other:?}"),
                    };
                    *work_ticket.lock().expect("ticket lock") = Some(queued);
                    Err::<i32, _>("task error")
                },
            )
            .await;
        wake.wait_finished().await;
        wake.assert_crossed(response_by);
        if cancel_during_release {
            assert!(matches!(
                result,
                Err(RunError::Governor(GovernorError::Cancelled))
            ));
        } else {
            assert!(matches!(
                result,
                Err(RunError::Governor(GovernorError::DeadlineExceeded))
            ));
        }
        let queued = ticket
            .lock()
            .expect("ticket lock")
            .take()
            .expect("ticket set");
        let permit = match rt.governor().claim(queued) {
            taskmesh::ext::ClaimOutcome::Ready(permit) => permit,
            other => panic!("promoted waiter not ready: {other:?}"),
        };
        assert!(matches!(
            rt.governor().release(permit),
            taskmesh::ext::ReleaseOutcome::Released
        ));
        assert_eq!(
            rt.snapshot().classes[&TaskClass::new("release-fence")].inflight,
            0
        );
    }
}
