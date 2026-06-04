//! T08: run_local is the local-runtime exception, not a general async path.

mod common;
use common::*;

use taskmesh::*;

#[tokio::test]
async fn run_local_rejects_non_local_classified_work() {
    let rt = runtime();
    // A blocking-classified spec must not reach the local runtime.
    let err = rt
        .run_local(blocking(), async { Ok::<i32, ()>(1) })
        .await
        .expect_err("non-local work rejected");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
}

#[tokio::test]
async fn run_local_accepts_local_classified_work() {
    let rt = runtime();
    let out: i32 = rt
        .run_local(local(), async { Ok::<_, ()>(99) })
        .await
        .expect("local work accepted");
    assert_eq!(out, 99);
}
