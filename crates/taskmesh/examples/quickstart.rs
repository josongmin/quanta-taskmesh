//! Quickstart — the 5-minute tour of the `taskmesh` facade.
//!
//! Run it: `cargo run -p taskmesh --example quickstart`
//!
//! `taskmesh` is a *governed execution control-plane*: you describe each unit of
//! work as a `TaskSpec` of some `TaskClass`, and the runtime admits, queues,
//! schedules, and accounts it according to per-class policy — instead of letting
//! work hit the executor unbounded.

use std::time::Duration;

use taskmesh::{
    Builder, ClassPolicy, FairnessPolicy, OverflowPolicy, ResourceBudget, RetryAfterPolicy,
    RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TopologyConfig,
};

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // 1. Build a runtime. Topology sizes the worker pools; ResourceBudget caps
    //    global cpu/memory units; each class declares its own governance policy.
    //    Invalid configs are rejected here (fail-closed), never mid-run.
    let runtime = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_auto() // CPU pool = available parallelism …
                .reserve_cores(1) // … minus reserved cores
                .blocking_threads(8),
        )
        .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(32) // at most 32 concurrent
                .max_queue_depth(128) // then queue up to 128
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .retry_after_policy(RetryAfterPolicy::Adaptive) // backpressure hint
                .fairness(FairnessPolicy::WeightedFairQueue {
                    weight: 4,
                    burst: 0,
                })
                .cpu_units(1)
                .memory_units(2),
        )
        .build()
        .expect("config is valid");

    let retrieval = TaskClass::new("retrieval");

    // 2. Run work on the substrate that fits it. The governor admits (or queues /
    //    rejects) before the closure ever runs; the permit is released after.
    //    Governor rejection and your task's own error stay distinct in RunError.
    let io: String = runtime
        .run_io(
            TaskSpec::io(retrieval.clone()).operation("search:repo:42"),
            async { Ok::<_, std::convert::Infallible>("io-result".to_string()) },
        )
        .await
        .expect("admitted + succeeded");
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("run_io      -> {io}");

    let blocking: u64 = runtime
        .run_blocking(
            TaskSpec::blocking(retrieval.clone()).operation("fsync:1"),
            || Ok::<_, std::convert::Infallible>(fib(30)),
        )
        .await
        .expect("ok");
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("run_blocking-> fib(30) = {blocking}");

    let cpu: u64 = runtime
        .run_cpu(
            TaskSpec::cpu(retrieval.clone()).operation("rank:shard:3"),
            || Ok::<_, std::convert::Infallible>(fib(32)),
        )
        .await
        .expect("ok");
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("run_cpu     -> fib(32) = {cpu}");

    // 3. Optional per-submission controls: pre-submit cancel + a bounded wait for
    //    a queued permit. A timed-out acquire is a typed governor-side error.
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(100));
    let bounded = runtime
        .run_blocking_with(
            TaskSpec::blocking(retrieval.clone()).operation("bounded"),
            opts,
            || Ok::<u8, ()>(7),
        )
        .await;
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("bounded     -> {bounded:?}");

    // 4. Errors: a task failure is RunError::Task, never flattened with a
    //    governor rejection.
    let failed: Result<(), RunError<&str>> = runtime
        .run_io(TaskSpec::io(retrieval.clone()).operation("boom"), async {
            Err::<(), _>("domain error")
        })
        .await;
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("task error  -> is_task={}", failed.unwrap_err().is_task());

    // 5. Observe governed state: per-class inflight/queued/held + substrate
    //    inventory. (All drained now, so inflight is 0.)
    let snap = runtime.snapshot();
    let c = snap
        .classes
        .get(&retrieval)
        .expect("retrieval class snapshot present");
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!(
        "snapshot    -> retrieval inflight={} queued={}, {} substrates registered",
        c.inflight,
        c.queued,
        snap.substrates.len()
    );

    // 6. Advanced integrators reach the governance engine directly via `ext`
    //    (e.g. to drive admission from a non-Tokio host, or audit provenance).
    let _governor: &taskmesh::ext::Governor = runtime.governor();
    // nosemgrep: taskmesh-no-terminal-io-in-library-crates -- example binary prints demo output for facade users.
    println!("ext         -> direct governor access available for embedders");
}

fn fib(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}
