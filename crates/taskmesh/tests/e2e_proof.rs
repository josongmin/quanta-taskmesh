//! Hell-gate end-to-end proof. Drives the public `taskmesh` facade through the
//! Closeout-Gate proof scenarios, with special focus on the integrated async
//! path that unit tests cannot reach: a queued request parking on a waker and
//! being promoted, woken, claimed, run, and released across real Tokio tasks.

use std::rc::Rc;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn retrieval_runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .memory_units(2)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .retry_after_policy(RetryAfterPolicy::Adaptive),
        )
        .build()
        .expect("runtime builds")
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("retrieval")).operation(op.to_string())
}

/// Poll the snapshot until `pred` holds for `class`, up to a generous budget.
/// Replaces fixed sleeps so sequencing assertions are robust on slow runners.
async fn wait_until(rt: &TokioRuntime, class: &str, pred: impl Fn(&ClassSnapshot) -> bool) {
    for _ in 0..500 {
        if pred(&rt.snapshot().classes[&TaskClass::new(class.to_string())]) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition for class {class} never reached (5s)");
}

/// THE hell-gate: a second request must queue while the first holds the only
/// slot, then be promoted and completed once the first releases — all through
/// the public async facade, across real worker threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_request_is_promoted_and_completed_end_to_end() {
    let rt = retrieval_runtime();
    let (release_first, hold) = tokio::sync::oneshot::channel::<()>();

    // Task A occupies the single inflight slot and blocks until signaled.
    let rt_a = rt.clone();
    let a = tokio::spawn(async move {
        rt_a.run_blocking(spec("op-a"), move || {
            // Block the worker thread until the test releases it.
            let _ = hold.blocking_recv();
            Ok::<_, ()>("a")
        })
        .await
    });

    // Let A get admitted (inflight == 1) before B arrives.
    wait_until(&rt, "retrieval", |c| c.inflight == 1).await;

    // Task B must queue (slot full), park on its waker, and wait.
    let rt_b = rt.clone();
    let b = tokio::spawn(async move { rt_b.run_blocking(spec("op-b"), || Ok::<_, ()>("b")).await });

    wait_until(&rt, "retrieval", |c| c.queued == 1).await;

    // Release A -> release() promotes B -> waker fires -> B claims, runs, finishes.
    release_first.send(()).unwrap();

    let (ra, rb) = tokio::join!(a, b);
    assert_eq!(ra.unwrap().unwrap(), "a");
    assert_eq!(rb.unwrap().unwrap(), "b", "queued request must complete");

    // Drained: nothing inflight or queued afterward.
    let snap = rt.snapshot();
    let c = &snap.classes[&TaskClass::new("retrieval")];
    assert_eq!((c.inflight, c.queued), (0, 0));
}

/// Proof scenario 1, 9: unknown class rejects through the facade; run_local
/// non-`Send` path works and is guarded.
#[tokio::test]
async fn unknown_class_and_run_local_paths() {
    let rt = retrieval_runtime();

    let unknown = TaskSpec::blocking(TaskClass::new("ghost")).operation("x");
    let err = rt
        .run_blocking(unknown, || Ok::<i32, ()>(1))
        .await
        .expect_err("unknown class rejects");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::UnknownClass { .. }
        ))
    ));

    // run_local requires a local-runtime-classified spec and accepts non-Send work.
    let local_spec = TaskSpec::local(TaskClass::new("retrieval")).operation("local");
    let out: i32 = rt
        .run_local(local_spec, async {
            let rc = Rc::new(41); // !Send
            Ok::<_, ()>(*rc + 1)
        })
        .await
        .expect("local work runs");
    assert_eq!(out, 42);

    // A non-local spec is refused by the guard.
    let bad = rt
        .run_local(spec("not-local"), async { Ok::<i32, ()>(0) })
        .await
        .expect_err("non-local rejected");
    assert!(matches!(
        bad,
        RunError::Governor(GovernorError::LocalRuntimeUnavailable)
    ));
}

/// Proof scenarios 2, 3, 4: inflight + queue-depth saturation and retry-after,
/// observed through the shared governor under the facade.
#[test]
fn saturation_and_retry_after_via_governor() {
    let rt = retrieval_runtime();
    let g = rt.governor();

    // Fill the one inflight slot.
    let held = match g.admit(&spec("a"), RequestKey::new("a")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("{other:?}"),
    };
    // Fill the queue (depth 8).
    for i in 0..8 {
        assert!(matches!(
            g.admit(&spec(&format!("q{i}")), RequestKey::new(format!("q{i}"))),
            AdmissionDecision::Queued { .. }
        ));
    }
    // Queue full -> QueueFull with an adaptive retry-after hint.
    match g.admit(&spec("overflow"), RequestKey::new("overflow")) {
        AdmissionDecision::Rejected(verdict @ AdmissionVerdict::QueueFull { .. }) => {
            assert!(verdict.retry_after_ms().is_some(), "adaptive hint present");
        }
        other => panic!("expected QueueFull, got {other:?}"),
    }
    g.release(held);
}

/// Proof scenarios 5, 6, 7, 8: memory overcommit, child→root attribution,
/// recursive rejection, and deterministic reduce enforcement.
#[test]
fn governance_invariants_via_governor() {
    // Memory overcommit reject.
    let rt = Builder::new()
        .resources(ResourceBudget::new().memory_units(10))
        .class_policy(
            TaskClass::new("heavy"),
            ClassPolicy::new().max_inflight(8).memory_units(10),
        )
        .build()
        .unwrap();
    let g = rt.governor();
    let big = TaskSpec::blocking(TaskClass::new("heavy")).operation("h1");
    assert!(matches!(
        g.admit(&big, RequestKey::new("h1")),
        AdmissionDecision::Admitted { .. }
    ));
    let big2 = TaskSpec::blocking(TaskClass::new("heavy")).operation("h2");
    assert!(matches!(
        g.admit(&big2, RequestKey::new("h2")),
        AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
    ));

    // Child attribution + recursive rejection.
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64))
        .class_policy(
            TaskClass::new("worker"),
            ClassPolicy::new().max_inflight(8).cpu_units(2),
        )
        .build()
        .unwrap();
    let g = rt.governor();
    let child = TaskSpec::blocking(TaskClass::new("worker")).child_of("root", TaskStage::new("p"));
    assert!(matches!(
        g.admit(&child, RequestKey::new("root")),
        AdmissionDecision::Admitted { .. }
    ));
    let root = g.root_attribution("root").expect("root tracked");
    assert_eq!(root.child_inflight, 1);
    assert_eq!(root.cpu_units, 2);
    // Same (root, stage) re-entry is a recursive loop -> reject.
    assert!(matches!(
        g.admit(&child, RequestKey::new("root")),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));

    // Deterministic reduce enforcement.
    let bad = TaskSpec::cpu(TaskClass::new("worker"))
        .fan_out_stage(TaskStage::new("merge"), SubstrateHint::SharedCpuExecutor);
    assert!(Governor::validate_reduce(&bad).is_err());
    let good = TaskSpec::cpu(TaskClass::new("worker")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("doc_id"),
    );
    assert!(Governor::validate_reduce(&good).is_ok());
}

/// Proof scenarios 10, 11: snapshot exposes class + substrate inventory; an
/// impossible budget is rejected at construction.
#[test]
fn snapshot_inventory_and_fail_closed_config() {
    let rt = retrieval_runtime();
    let snap = rt.snapshot();
    assert!(snap.classes.contains_key(&TaskClass::new("retrieval")));
    let substrate_names: Vec<String> = snap.substrates.iter().map(|s| s.name.to_string()).collect();
    for builtin in BUILTIN_SUBSTRATES {
        assert!(
            substrate_names.contains(&builtin.to_string()),
            "missing {builtin}"
        );
    }

    // Impossible budget: class cost exceeds the global ceiling -> build fails.
    let err = Builder::new()
        .resources(ResourceBudget::new().cpu_units(4))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().cpu_units(8), // exceeds the 4-unit budget
        )
        .build();
    assert!(matches!(err, Err(GovernorError::PolicyViolation(_))));
}
