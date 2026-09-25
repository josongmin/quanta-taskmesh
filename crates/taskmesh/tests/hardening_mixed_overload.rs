//! H01: a finite open-loop burst across the four host dispatch families must
//! be decided by the governor before an executor sees rejected work.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use taskmesh::*;

const HANG: Duration = Duration::from_secs(5);
const STACK: u64 = 2 * 1024 * 1024;
const CLASSES: [&str; 4] = ["io", "blocking", "cpu", "stack"];
const SHARED_HELD: u32 = if cfg!(feature = "rayon") { 1 } else { 2 };
const PHYSICAL_CPU_HELD: u32 = if cfg!(feature = "rayon") { 1 } else { 0 };

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    cv: Condvar,
}

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

// A failed assertion must not leave a Tokio blocking worker waiting forever.
struct ReleaseOnDrop(Arc<Gate>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn class(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

fn runtime() -> TokioRuntime {
    let mut builder = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_fixed(1)
                .blocking_threads(1)
                .large_stack_slots(1)
                .shared_blocking_domain(PhysicalDomainMode::Fixed(2)),
        )
        .resources(ResourceBudget::new().cpu_units(4).memory_units(4));
    for name in CLASSES {
        builder = builder.class_policy(
            class(name),
            ClassPolicy::new()
                .max_inflight(2)
                .max_queue_depth(0)
                .cpu_units(1)
                .memory_units(1)
                .overflow_policy(OverflowPolicy::Reject),
        );
    }
    builder.build().expect("finite mixed topology builds")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_substrate_open_loop_burst_never_reaches_workers_before_permit() {
    let rt = runtime();
    let gate = Arc::new(Gate::default());
    let release_guard = ReleaseOnDrop(Arc::clone(&gate));
    let (io_release, io_hold) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();

    let io = {
        let rt = rt.clone();
        let started = started_tx.clone();
        tokio::spawn(async move {
            rt.run_io(TaskSpec::io(class("io")).operation("held-io"), async move {
                started.send("io").expect("observer alive");
                let _ = io_hold.await;
                Ok::<(), ()>(())
            })
            .await
        })
    };
    let blocking = {
        let rt = rt.clone();
        let started = started_tx.clone();
        let gate = Arc::clone(&gate);
        tokio::spawn(async move {
            rt.run_blocking(
                TaskSpec::blocking(class("blocking")).operation("held-blocking"),
                move || {
                    started.send("blocking").expect("observer alive");
                    gate.wait();
                    Ok::<(), ()>(())
                },
            )
            .await
        })
    };
    let cpu = {
        let rt = rt.clone();
        let started = started_tx.clone();
        let gate = Arc::clone(&gate);
        tokio::spawn(async move {
            rt.run_cpu(
                TaskSpec::cpu(class("cpu")).operation("held-cpu"),
                move || {
                    started.send("cpu").expect("observer alive");
                    gate.wait();
                    Ok::<(), ()>(())
                },
            )
            .await
        })
    };
    let stack = {
        let rt = rt.clone();
        let started = started_tx;
        let gate = Arc::clone(&gate);
        tokio::spawn(async move {
            rt.run_blocking(
                TaskSpec::base(class("stack"), SubstrateHint::LargeStackCapability)
                    .operation("held-stack")
                    .stack_size_bytes(STACK),
                move || {
                    started.send("stack").expect("observer alive");
                    gate.wait();
                    Ok::<(), ()>(())
                },
            )
            .await
        })
    };

    let observed = tokio::time::timeout(HANG, async {
        let mut observed = BTreeSet::new();
        for _ in CLASSES {
            observed.insert(started_rx.recv().await.expect("holder starts"));
        }
        observed
    })
    .await
    .expect("all four substrate holders start");
    assert_eq!(observed, CLASSES.into_iter().collect());

    let held = rt.snapshot();
    for name in CLASSES {
        let state = &held.classes[&class(name)];
        assert_eq!((state.inflight, state.queued), (1, 0), "{name}");
        assert_eq!((state.cpu_units_held, state.memory_units_held), (1, 1));
        assert_eq!((state.admitted_total, state.started_total), (1, 1));
    }
    assert_eq!(
        held.capabilities[PHYSICAL_SHARED_BLOCKING].in_use,
        SHARED_HELD
    );
    assert_eq!(held.capabilities[PHYSICAL_CPU].in_use, PHYSICAL_CPU_HELD);
    assert_eq!(held.capabilities[PHYSICAL_DEDICATED].in_use, 1);
    assert_eq!(held.capabilities["blocking"].in_use, 1);
    assert_eq!(held.capabilities["cpu"].in_use, 1);
    assert_eq!(held.capabilities["large_stack"].in_use, 1);
    assert_eq!(held.conservation_violation(), None);

    // Offered concurrently while every CPU unit is owned by a live worker.
    // The first side effect of each closure/future is counted; zero proves no
    // rejected submission reached a worker or an executor-side hidden queue.
    let ran = Arc::new(AtomicUsize::new(0));
    let mut burst = Vec::new();
    for index in 0..16 {
        let rt = rt.clone();
        let ran = Arc::clone(&ran);
        burst.push(tokio::spawn(async move {
            let spec = match index % 4 {
                0 => TaskSpec::io(class("io")),
                1 => TaskSpec::blocking(class("blocking")),
                2 => TaskSpec::cpu(class("cpu")),
                _ => TaskSpec::base(class("stack"), SubstrateHint::LargeStackCapability)
                    .stack_size_bytes(STACK),
            }
            .operation(format!("burst-{index}"));
            match index % 4 {
                0 => {
                    rt.run_io(spec, async move {
                        ran.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    })
                    .await
                }
                1 => {
                    rt.run_blocking(spec, move || {
                        ran.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    })
                    .await
                }
                2 => {
                    rt.run_cpu(spec, move || {
                        ran.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    })
                    .await
                }
                _ => {
                    rt.run_blocking(spec, move || {
                        ran.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), ()>(())
                    })
                    .await
                }
            }
        }));
    }
    for (index, request) in burst.into_iter().enumerate() {
        let outcome = tokio::time::timeout(HANG, request)
            .await
            .expect("burst request receives a bounded decision")
            .expect("burst task joins");
        let verdict = outcome.as_ref().err().and_then(RunError::as_verdict);
        assert!(
            if index % 4 == 0 {
                matches!(verdict, Some(AdmissionVerdict::CpuSaturated { .. }))
            } else {
                matches!(verdict, Some(AdmissionVerdict::SubstrateSaturated { .. }))
            },
            "request {index} must report its exact saturation cause: {outcome:?}"
        );
    }
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    let saturated = rt.snapshot();
    for name in CLASSES {
        let state = &saturated.classes[&class(name)];
        assert_eq!((state.inflight, state.queued), (1, 0), "{name}");
        assert_eq!((state.cpu_units_held, state.memory_units_held), (1, 1));
        assert_eq!((state.admitted_total, state.started_total), (1, 1));
    }
    assert_eq!(
        saturated.capabilities[PHYSICAL_SHARED_BLOCKING].in_use,
        SHARED_HELD
    );
    assert_eq!(
        saturated.capabilities[PHYSICAL_CPU].in_use,
        PHYSICAL_CPU_HELD
    );
    assert_eq!(saturated.capabilities[PHYSICAL_DEDICATED].in_use, 1);
    assert_eq!(saturated.capabilities["blocking"].in_use, 1);
    assert_eq!(saturated.capabilities["cpu"].in_use, 1);
    assert_eq!(saturated.capabilities["large_stack"].in_use, 1);
    assert_eq!(saturated.conservation_violation(), None);

    io_release.send(()).expect("IO holder still waiting");
    release_guard.0.release();
    for holder in [io, blocking, cpu, stack] {
        tokio::time::timeout(HANG, holder)
            .await
            .expect("holder completes after release")
            .expect("holder joins")
            .expect("holder succeeds");
    }
    let drained = rt.snapshot();
    for name in CLASSES {
        let state = &drained.classes[&class(name)];
        assert_eq!((state.inflight, state.queued), (0, 0), "{name}");
        assert_eq!((state.cpu_units_held, state.memory_units_held), (0, 0));
    }
    assert_eq!(drained.capabilities[PHYSICAL_SHARED_BLOCKING].in_use, 0);
    assert_eq!(drained.capabilities[PHYSICAL_CPU].in_use, 0);
    assert_eq!(drained.capabilities[PHYSICAL_DEDICATED].in_use, 0);
    assert_eq!(drained.capabilities["blocking"].in_use, 0);
    assert_eq!(drained.capabilities["cpu"].in_use, 0);
    assert_eq!(drained.capabilities["large_stack"].in_use, 0);
    assert_eq!(drained.conservation_violation(), None);

    rt.run_io(TaskSpec::io(class("io")).operation("after-drain"), async {
        Ok::<(), ()>(())
    })
    .await
    .expect("capacity must be reusable after all holders finish");
}
