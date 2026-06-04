//! Allocation probe (ADR 9000 / P2 allocation track): the deterministic count of
//! heap allocations per admit→release cycle. This is the *characterization*
//! metric whose CI gate is "no regression vs baseline" — distinct from the
//! 0-alloc storage-model goal, which requires a separate redesign (root-id
//! interning / `Arc<str>`), tracked apart from this benchmark.
//!
//! Run: `cargo run -p taskmesh-bench --example alloc_probe --release`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use taskmesh_bench::workload::{fixture, root_spec};
use taskmesh_contract::ClassPolicy;
use taskmesh_engine::AdmissionDecision;

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
    // Cost 0 / budgets 0 → admit always succeeds; reused spec + statically-keyed
    // RequestKey so the only allocations counted are the governor's own.
    let fx = fixture(
        vec![("retrieval", ClassPolicy::new().cpu_units(1).memory_units(1))],
        0,
        0,
    );
    let g = &fx.governor;
    let spec = root_spec("retrieval", "search:repo:1");

    let cycle = |g: &taskmesh_engine::Governor| {
        if let AdmissionDecision::Admitted { permit_id } = g.admit(&spec) {
            g.release(permit_id);
        }
    };

    for _ in 0..1_000 {
        cycle(g); // warm allocator / amortize first-touch growth
    }

    let n = 200_000u64;
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..n {
        cycle(g);
    }
    let after = ALLOCS.load(Ordering::Relaxed);

    let per_op = (after - before) as f64 / n as f64;
    println!("admit+release allocations/op = {per_op:.3}");
    println!(
        "(baseline characterization; 0-alloc is a separate storage-model goal — ADR 9000 / P2)"
    );
}
