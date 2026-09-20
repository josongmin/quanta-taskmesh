//! H03: physical worker domains are registered, finite, and charged atomically
//! with semantic role capabilities. The fixture uses channels/condvars only;
//! no scheduler sleeps or yield-based assertions.

#[cfg(not(feature = "rayon"))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(feature = "rayon"))]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(not(feature = "rayon"))]
use std::{future::Future, task::Poll};

use taskmesh::*;

#[derive(Default)]
#[cfg(not(feature = "rayon"))]
struct Gate {
    released: Mutex<bool>,
    cv: Condvar,
}

#[cfg(not(feature = "rayon"))]
impl Gate {
    fn wait(&self) {
        let mut released = self.released.lock().expect("gate lock");
        while !*released {
            released = self.cv.wait(released).expect("gate wait");
        }
    }

    fn release(&self) {
        *self.released.lock().expect("gate lock") = true;
        self.cv.notify_all();
    }
}

// An assertion failure must still unblock the worker closures before Tokio
// tears down its blocking pool. Otherwise a useful failure becomes a hang.
#[cfg(not(feature = "rayon"))]
struct ReleaseOnDrop(Arc<Gate>);

#[cfg(not(feature = "rayon"))]
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(not(feature = "rayon"))]
fn track_peak(active: &AtomicUsize, peak: &AtomicUsize) {
    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
    peak.fetch_max(current, Ordering::SeqCst);
}

#[cfg(not(feature = "rayon"))]
fn job(
    started: tokio::sync::mpsc::UnboundedSender<()>,
    gate: Arc<Gate>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
) -> impl FnOnce() -> Result<(), ()> + Send + 'static {
    move || {
        track_peak(&active, &peak);
        started.send(()).expect("observer alive");
        gate.wait();
        active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[cfg(not(feature = "rayon"))]
async fn blocking_maintenance_and_cpu_fallback_share_one_finite_domain() {
    let class = TaskClass::new("c");
    let rt = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_fixed(4)
                .shared_blocking_domain(PhysicalDomainMode::Fixed(2)),
        )
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            class.clone(),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("finite physical topology builds");

    let names: Vec<_> = rt
        .config()
        .substrates
        .iter()
        .filter_map(|record| record.capability_pool.as_deref())
        .collect();
    assert!(names.contains(&PHYSICAL_SHARED_BLOCKING));
    assert_eq!(
        rt.snapshot().capabilities[PHYSICAL_SHARED_BLOCKING].limit,
        2,
        "declared domain and snapshot authority must agree"
    );

    let gate = Arc::new(Gate::default());
    let release_on_drop = ReleaseOnDrop(Arc::clone(&gate));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();

    let blocking = {
        let rt = rt.clone();
        let run = job(
            started_tx.clone(),
            Arc::clone(&gate),
            Arc::clone(&active),
            Arc::clone(&peak),
        );
        tokio::spawn(async move {
            rt.run_blocking(
                TaskSpec::blocking(TaskClass::new("c")).operation("blocking"),
                run,
            )
            .await
        })
    };
    let maintenance = {
        let rt = rt.clone();
        let run = job(
            started_tx.clone(),
            Arc::clone(&gate),
            Arc::clone(&active),
            Arc::clone(&peak),
        );
        tokio::spawn(async move {
            rt.run_blocking(
                TaskSpec::base(TaskClass::new("c"), SubstrateHint::BackgroundOnly)
                    .operation("maintenance"),
                run,
            )
            .await
        })
    };
    started_rx.recv().await.expect("first worker starts");
    started_rx.recv().await.expect("second worker starts");
    // Poll the third submission ourselves after both physical slots are known
    // to be occupied. A spawned future alone gives no ordering guarantee that
    // admission has happened by the time the snapshot is read.
    let mut cpu = Box::pin(rt.run_cpu(
        TaskSpec::cpu(TaskClass::new("c")).operation("cpu"),
        job(
            started_tx,
            Arc::clone(&gate),
            Arc::clone(&active),
            Arc::clone(&peak),
        ),
    ));
    std::future::poll_fn(|cx| {
        assert!(cpu.as_mut().poll(cx).is_pending(), "third job must queue");
        Poll::Ready(())
    })
    .await;
    let saturated = rt.snapshot();
    assert_eq!(saturated.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 2);
    assert_eq!(saturated.classes[&class].queued, 1);
    assert_eq!(peak.load(Ordering::SeqCst), 2);

    release_on_drop.0.release();
    for handle in [blocking, maintenance] {
        handle.await.expect("join").expect("job succeeds");
    }
    cpu.await.expect("cpu job succeeds");
    assert_eq!(peak.load(Ordering::SeqCst), 2, "aggregate bound held");
    let drained = rt.snapshot();
    assert_eq!(drained.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 0);
    assert_eq!(drained.classes[&class].inflight, 0);
    assert_eq!(drained.classes[&class].queued, 0);
    assert_eq!(drained.conservation_violation(), None);
}

#[cfg(feature = "rayon")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rayon_cpu_domain_is_separate_and_observable() {
    let rt = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_fixed(1)
                .shared_blocking_domain(PhysicalDomainMode::Fixed(2)),
        )
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(4).cpu_units(1),
        )
        .build()
        .expect("rayon topology builds");
    let descriptor = rt.executor_capabilities();
    assert_eq!(descriptor.physical_domain, Some(PHYSICAL_CPU));
    assert_eq!(descriptor.declared_workers, Some(1));
    assert!(descriptor.exclusive_pool);
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.capabilities[PHYSICAL_CPU].limit, 1);
    assert_eq!(snapshot.capabilities[PHYSICAL_SHARED_BLOCKING].limit, 2);

    let result: i32 = rt
        .run_cpu(TaskSpec::cpu(TaskClass::new("c")).operation("cpu"), || {
            Ok::<_, ()>(7)
        })
        .await
        .expect("rayon job");
    assert_eq!(result, 7);
    let drained = rt.snapshot();
    assert_eq!(drained.capabilities[PHYSICAL_CPU].in_use, 0);
    assert_eq!(drained.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 0);
}

#[test]
fn zero_fixed_physical_domains_are_typed_build_errors() {
    let error = Builder::new()
        .topology(TopologyConfig::new().shared_blocking_domain(PhysicalDomainMode::Fixed(0)))
        .build()
        .expect_err("zero physical domain cannot run work");
    assert_eq!(
        error,
        GovernorError::InvalidTopology(TopologyError::ZeroFixedPhysicalDomain {
            domain: PHYSICAL_SHARED_BLOCKING,
        })
    );
}
