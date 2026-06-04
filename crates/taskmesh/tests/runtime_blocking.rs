//! T07: run_blocking happy path; a join failure is a governor/runtime-side error.

mod common;
use common::*;

use taskmesh::*;

#[tokio::test]
async fn run_blocking_happy_path() {
    let rt = runtime();
    let out: String = rt
        .run_blocking(blocking(), || {
            Ok::<_, std::convert::Infallible>("ok".to_string())
        })
        .await
        .expect("blocking succeeds");
    assert_eq!(out, "ok");
}

#[tokio::test]
async fn run_blocking_task_error_stays_task() {
    let rt = runtime();
    let err = rt
        .run_blocking(blocking(), || Err::<i32, _>("task-fail"))
        .await
        .expect_err("task fails");
    assert!(matches!(err, RunError::Task("task-fail")));
}

#[tokio::test]
async fn join_failure_is_governor_side() {
    let rt = runtime();
    // A panic in the blocking job surfaces as a JoinError -> governor-side error,
    // never as a task error.
    let err = rt
        .run_blocking(blocking(), || -> Result<i32, ()> { panic!("kaboom") })
        .await
        .expect_err("join fails");
    assert!(
        err.is_governor(),
        "join failure must be governor-side: {err:?}"
    );
}
