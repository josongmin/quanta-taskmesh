//! T07: run_cpu happy path through the default (blocking-pool) CPU executor.

mod common;
use common::*;

use taskmesh::*;

#[tokio::test]
async fn run_cpu_happy_path() {
    let rt = runtime();
    let out: i32 = rt
        .run_cpu(cpu(), || Ok::<_, ()>(21 * 2))
        .await
        .expect("cpu work succeeds");
    assert_eq!(out, 42);
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test]
async fn run_cpu_task_error_stays_task() {
    let rt = runtime();
    let err = rt
        .run_cpu(cpu(), || Err::<i32, _>("cpu-fail"))
        .await
        .expect_err("task fails");
    assert!(matches!(err, RunError::Task("cpu-fail")));
}
