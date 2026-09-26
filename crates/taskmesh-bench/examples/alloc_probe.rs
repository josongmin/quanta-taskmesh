//! Allocation probe (ADR 9000 / P2 allocation track): the deterministic count of
//! heap allocations per admit→release cycle. This is the *characterization*
//! metric whose CI gate is "no regression vs baseline" — distinct from the
//! 0-alloc storage-model goal, which requires a separate redesign (root-id
//! interning / `Arc<str>`), tracked apart from this benchmark.
//!
//! # What one "op" is
//!
//! An op is one **admitted** `admit` followed by its `release`. A cycle that is
//! queued or rejected is not a cheaper op; it is a different operation, and
//! counting it under the same denominator would report a lower cost for a
//! regression that stopped admitting. So the timed loop refuses any verdict but
//! `Admitted`, the probe checks that `completed == attempted`, and the final
//! ledger must be empty.
//!
//! # Output contract
//!
//! One machine-readable line, consumed by `tools/bench-gate.sh`:
//!
//! ```text
//! taskmesh-alloc-probe schema=3 attempted=<n> completed=<n> total_allocations=<n> allocs_per_op=<f> final_inflight=<n> counter_check=<n>/<n>
//! ```
//!
//! `counter_check` is the instrument's own self-test: before measuring, the
//! probe performs a known number of heap allocations and reports how many the
//! counter saw. A counter that under-reports would otherwise make a regression
//! look like an improvement, and a counter reporting `0` would pass any gate.
//!
//! Human-facing text goes to stderr so it cannot be mistaken for the metric.
//!
//! Run: `cargo run -p taskmesh-bench --example alloc_probe --release`
//!
//! `TASKMESH_ALLOC_PROBE_FORCE=reject` makes the fixture reject every admission,
//! so the "not an op" exit path can be driven end to end by the gate's tests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use taskmesh_bench::workload::{fixture, root_spec};
use taskmesh_contract::{ClassPolicy, TaskClass};
use taskmesh_engine::{AdmissionDecision, ReleaseOutcome};

/// Bumped when the measurement definition changes. A baseline recorded under a
/// different schema is not comparable to this output.
pub const MEASUREMENT_SCHEMA: u32 = 3;

/// Allocations the self-check performs; the counter must see exactly this many.
const COUNTER_CHECK_ALLOCS: u64 = 1_000;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() {
    // Instrument self-check first: a counter that cannot see a known number of
    // allocations cannot be trusted to see the governor's.
    let before = ALLOCS.load(Ordering::Relaxed);
    for i in 0..COUNTER_CHECK_ALLOCS {
        let boxed = Box::new(i);
        std::hint::black_box(&boxed);
        drop(boxed);
    }
    let counted = ALLOCS.load(Ordering::Relaxed) - before;
    if counted != COUNTER_CHECK_ALLOCS {
        eprintln!(
            "taskmesh-alloc-probe: counter self-check failed: {counted} of {COUNTER_CHECK_ALLOCS} allocations were counted"
        );
        std::process::exit(2);
    }

    // Cost 0 / budgets 0 → admit always succeeds; reused spec + statically-keyed
    // RequestKey so the only allocations counted are the governor's own.
    //
    // The forced-reject variant (a disabled class) exists so the gate's own
    // tests can drive the "not an op" exit path through the real producer.
    let class = match std::env::var("TASKMESH_ALLOC_PROBE_FORCE").as_deref() {
        Ok("reject") => ClassPolicy::new().max_inflight(0),
        Ok(other) => {
            eprintln!("taskmesh-alloc-probe: unknown TASKMESH_ALLOC_PROBE_FORCE={other:?}");
            std::process::exit(2);
        }
        Err(_) => ClassPolicy::new().cpu_units(1).memory_units(1),
    };
    let fx = fixture(vec![("retrieval", class)], 0, 0);
    let g = &fx.governor;
    let spec = root_spec("retrieval", "search:repo:1");

    // Every cycle must be a full admit→release; anything else is a fixture or
    // implementation regression and the probe stops rather than measuring it.
    let cycle = |g: &taskmesh_engine::Governor| -> u64 {
        match g.admit(&spec) {
            AdmissionDecision::Admitted { permit_id } => {
                assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
                1
            }
            other => {
                eprintln!("taskmesh-alloc-probe: admission was not Admitted: {other:?}");
                std::process::exit(2);
            }
        }
    };

    for _ in 0..1_000 {
        cycle(g); // warm allocator / amortize first-touch growth
    }

    let attempted = 200_000u64;
    let before = ALLOCS.load(Ordering::Relaxed);
    let mut completed = 0u64;
    for _ in 0..attempted {
        completed += cycle(g);
    }
    let after = ALLOCS.load(Ordering::Relaxed);

    let final_inflight = g.snapshot().classes[&TaskClass::new("retrieval")].inflight;
    if completed != attempted || final_inflight != 0 {
        eprintln!(
            "taskmesh-alloc-probe: post-condition failed: completed={completed} attempted={attempted} final_inflight={final_inflight}"
        );
        std::process::exit(2);
    }

    let total_allocations = after - before;
    let per_op = total_allocations as f64 / completed as f64;
    println!(
        "taskmesh-alloc-probe schema={MEASUREMENT_SCHEMA} attempted={attempted} completed={completed} total_allocations={total_allocations} allocs_per_op={per_op:.3} final_inflight={final_inflight} counter_check={counted}/{COUNTER_CHECK_ALLOCS}"
    );
    eprintln!(
        "(baseline characterization; 0-alloc is a separate storage-model goal — ADR 9000 / P2)"
    );
}
