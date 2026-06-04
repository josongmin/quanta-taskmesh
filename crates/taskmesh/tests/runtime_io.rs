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
    // permit released: nothing inflight afterwards
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
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
