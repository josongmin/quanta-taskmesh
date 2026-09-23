//! T07: run_blocking happy path. Typed worker failures and task-error custody
//! are covered with permit/recovery assertions in host_inferno.

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
