//! The surface a consumer sees through the `taskmesh` facade alone (audit
//! round 3, external-consumer lens). Each item here was a real stumbling block
//! for a consumer written from the public docs: a snapshot key that appeared
//! and vanished with occupancy, a type it could not name, a value it could not
//! read, an outcome it could drop without a warning, a runtime it could not
//! `unwrap_err()` past because it had no `Debug`.

use std::sync::Arc;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(2).blocking_threads(2))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(4)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

#[tokio::test]
async fn every_registered_pool_is_always_a_snapshot_key() {
    // A dashboard indexes `capabilities["maintenance"]`; before, an ungated
    // pool was present only while a job held it, so the key flapped.
    let rt = runtime();
    let idle = rt.snapshot();
    for pool in [
        "cpu",
        "blocking",
        "large_stack",
        "local_runtime",
        "maintenance",
    ] {
        let usage = idle
            .capabilities
            .get(pool)
            .unwrap_or_else(|| panic!("{pool} must be a key while idle: {:?}", idle.capabilities));
        assert_eq!(usage.in_use, 0, "{pool}");
    }
    assert_eq!(idle.capabilities["cpu"].limit, 2, "gated by topology");
    assert_eq!(idle.capabilities["blocking"].limit, 2);
    assert_eq!(
        idle.capabilities["maintenance"].limit, 0,
        "ungated pools carry limit 0"
    );

    // While a maintenance job holds the pool, the same key reports it — and
    // the key survives the release.
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let rt_job = rt.clone();
    let job = tokio::spawn(async move {
        rt_job
            .run_blocking(
                TaskSpec::base(TaskClass::new("c"), SubstrateHint::BackgroundOnly).operation("m"),
                move || {
                    // A signal or a dropped sender both release the holder.
                    let _released = hold.recv();
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rt.snapshot().capabilities["maintenance"].in_use == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "maintenance job never started"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    release.send(()).expect("job still waiting");
    job.await.expect("join").expect("job ok");
    let after = rt.snapshot();
    assert_eq!(after.capabilities["maintenance"].in_use, 0);
    assert_eq!(
        after.capabilities.len(),
        idle.capabilities.len(),
        "no key came or went"
    );
}

#[test]
fn a_pending_request_key_is_readable_through_the_facade() {
    // `PendingView::request_key` is a `RequestKey`; a consumer must be able to
    // name the type and read it without depending on `taskmesh_engine`.
    let rt = runtime();
    let g = rt.governor();
    let spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let AdmissionDecision::Admitted { permit_id } = g.admit(&spec) else {
        panic!("first admission");
    };
    let AdmissionDecision::Queued { ticket } = g.admit(&spec) else {
        panic!("second queues");
    };
    let view = g.pending_view(ticket).expect("queued");
    let key: &RequestKey = &view.request_key;
    assert_eq!(key.as_str(), spec.root_operation_id);
    assert_eq!(key.to_string(), spec.root_operation_id);
    assert_eq!(key, &RequestKey::from_root(&spec.root_operation_id));
    g.abandon(ticket);
    assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
}

#[test]
fn the_runtime_and_builder_are_debug_and_the_deadline_is_comparable() {
    // `Result<TokioRuntime, _>::unwrap_err()` needs `TokioRuntime: Debug`; the
    // CHANGELOG's own migration example did not compile without it.
    let error = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .cpu_executor(Arc::new(NarrowExecutor))
        .build()
        .unwrap_err();
    assert_eq!(
        error,
        GovernorError::InvalidTopology(TopologyError::ExecutorDeclaresFewerWorkers {
            declared: 1,
            resolved: 4,
        })
    );
    let rt = runtime();
    let printed = format!("{rt:?}");
    assert!(
        printed.contains("TokioRuntime") && printed.contains("cpu_executor"),
        "{printed}"
    );
    let printed = format!(
        "{:?}",
        Builder::new().topology(TopologyConfig::new().cpu_fixed(1))
    );
    assert!(printed.contains("Builder"), "{printed}");
    let printed = format!("{:?}", rt.governor());
    assert!(
        printed.contains("Governor") && printed.contains("classes"),
        "{printed}"
    );
    // `SubmitOptions.deadline` can be compared, so a caller can assert what it set.
    let opts = SubmitOptions::unbounded().with_deadline(Duration::from_secs(2));
    assert_eq!(
        opts.deadline,
        Some(SubmissionDeadline::RunFor(Duration::from_secs(2)))
    );
    assert_ne!(
        opts.deadline,
        Some(SubmissionDeadline::RunFor(Duration::from_secs(3)))
    );
    // The facade names the topology slot ceiling the docs tell a consumer to
    // pre-validate against.
    assert_eq!(MAX_CAPABILITY_SLOTS, u32::MAX as usize);
}

struct NarrowExecutor;

impl CpuExecutor for NarrowExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy().declared_workers(1)
    }
}
