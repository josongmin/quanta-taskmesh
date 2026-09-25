//! T07: run_io happy path and the governor/task error split.

mod common;
use common::*;

use taskmesh::*;

#[tokio::test]
async fn run_io_happy_path() {
    let rt = runtime();
    let out: i32 = rt
        .run_io(io(), async { Ok::<_, ()>(7) })
        .await
        .expect("io succeeds");
    assert_eq!(out, 7);
    let snapshot = rt.snapshot();
    let class = &snapshot.classes[&TaskClass::new("c")];
    assert_eq!(
        (
            class.inflight,
            class.queued,
            class.dispatch_reserved,
            class.accepted,
            class.running,
            class.cleanup_pending,
        ),
        (0, 0, 0, 0, 0, 0),
        "a successful IO result must return every phase to idle"
    );
    assert!(
        snapshot
            .capabilities
            .values()
            .all(|capability| capability.in_use == 0),
        "run_io must not leave any capability reservation"
    );
    assert_eq!(snapshot.conservation_violation(), None);
}

#[tokio::test]
async fn task_error_stays_task() {
    let rt = runtime();
    let err = rt
        .run_io(io(), async { Err::<i32, _>("boom") })
        .await
        .expect_err("task fails");
    assert!(
        err.is_task(),
        "task error must not flatten into governor error"
    );
    assert!(matches!(err, RunError::Task("boom")));
}
