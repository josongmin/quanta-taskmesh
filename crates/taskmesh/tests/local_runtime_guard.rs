//! T08: run_local is the local-runtime exception, not a general async path.

mod common;
use common::*;

use std::cell::Cell;
use std::rc::Rc;
use taskmesh::*;

#[tokio::test]
async fn run_local_rejects_non_local_classified_work() {
    let rt = runtime();
    let ran = Rc::new(Cell::new(false));
    let ran_in_work = Rc::clone(&ran);
    // A blocking-classified spec must not reach the local runtime.
    let err = rt
        .run_local(blocking(), async move {
            ran_in_work.set(true);
            Ok::<i32, ()>(1)
        })
        .await
        .expect_err("non-local work rejected");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
    assert!(!ran.get());
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}
