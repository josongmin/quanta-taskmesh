//! T07: run_local accepts a non-`Send` future on the local-runtime substrate.

mod common;
use common::*;

use std::rc::Rc;

use taskmesh::*;

#[tokio::test]
async fn run_local_with_non_send_future() {
    let rt = runtime();
    let out: i32 = rt
        .run_local(local(), async {
            // `Rc` is not `Send`; this future can only run on a local set.
            let value = Rc::new(41);
            let bumped = *value + 1;
            Ok::<_, ()>(bumped)
        })
        .await
        .expect("local work succeeds");
    assert_eq!(out, 42);
}
