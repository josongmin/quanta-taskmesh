//! Inferno: adversarial invariant gates — strictly harder than `hellgate`.
//!
//! Where `hellgate` validates the simulator/metrics, `inferno` attacks the
//! governor *directly* and tries to break the safety invariants the design
//! promises: capacity is never exceeded, resources are conserved across every
//! op order, fail-closed paths reject (not default-admit), the recursion guard
//! and leak sweep are exact, and lifecycle ops are idempotent under abuse.
//!
//! Every assertion here is true *by design* of `taskmesh-engine`. A failure is a
//! real correctness regression, not a flaky timing — these are deterministic.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use taskmesh_bench::workload::{fixture, retrieval_policy, root_spec};
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, DeterministicReducePolicy, MemoryOvercommitPolicy,
    MemoryReleasePolicy, OverflowPolicy, RetryAfterPolicy, Snapshot, SubstrateHint, TaskClass,
    TaskSpec, TaskStage,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, Ticket};

// ---- shared invariant helpers ---------------------------------------------

fn cpu_held(snap: &Snapshot) -> u32 {
    snap.classes.values().map(|c| c.cpu_units_held).sum()
}
fn mem_held(snap: &Snapshot) -> u32 {
    snap.classes.values().map(|c| c.memory_units_held).sum()
}
fn inflight(snap: &Snapshot, class: &str) -> u32 {
    snap.classes
        .get(&TaskClass::new(class.to_string()))
        .map(|c| c.inflight)
        .unwrap_or(0)
}

/// Claim every queued ticket the governor has already promoted, moving it from
/// `pending` to `held`. Mirrors the host's promote→claim handoff.
fn drain_promotions(g: &Governor, pending: &mut Vec<Ticket>, held: &mut Vec<PermitId>) {
    let mut i = 0;
    while i < pending.len() {
        if let Some(permit) = g.claim(pending[i]) {
            held.push(permit);
            pending.swap_remove(i);
        } else {
            i += 1;
        }
    }
}

// ===========================================================================
// 1. Randomized conservation fuzzer — the brutal one.
// ===========================================================================

/// Drive a random adversarial stream of admit/release/abandon/claim/reconcile/
/// stage-release ops at the real governor across many seeds. After *every* op
/// the hard caps must hold; after a full drain *everything* must return to zero
/// with no permit leaked, double-freed, or vanished.
#[test]
fn fuzz_conservation_and_caps_under_adversarial_churn() {
    const CPU_BUDGET: u32 = 8;
    let classes = ["retrieval", "rank", "index"];
    let max_inflight = |c: &str| match c {
        "retrieval" => 4u32,
        "rank" => 2,
        "index" => 3,
        _ => unreachable!(),
    };
    let max_depth = |c: &str| match c {
        "retrieval" => 16usize,
        "rank" => 8,
        "index" => 8,
        _ => unreachable!(),
    };

    for &seed in &[1u64, 2, 3, 7, 42, 0xC0FFEE, 0xDEAD_BEEF, 0x1234_5678] {
        let fx = fixture(
            vec![
                ("retrieval", retrieval_policy(4, 16)),
                ("rank", retrieval_policy(2, 8)),
                ("index", retrieval_policy(3, 8)),
            ],
            CPU_BUDGET,
            0, // memory budget disabled: cpu + inflight are the binding limits
        );
        let g = &*fx.governor;
        let mut rng = StdRng::seed_from_u64(seed);
        let mut held: Vec<PermitId> = Vec::new();
        let mut pending: Vec<Ticket> = Vec::new();
        let mut clock_ms = 0u64;
        let mut op_id = 0u64;

        for _ in 0..2_000 {
            if rng.gen_bool(0.2) {
                clock_ms += rng.gen_range(1..50);
                fx.clock.set(clock_ms);
            }
            match rng.gen_range(0u8..6) {
                0 => {
                    // ADMIT a fresh root request on a random class.
                    op_id += 1;
                    let class = classes[rng.gen_range(0..classes.len())];
                    let op = format!("op-{op_id}");
                    match g.admit(&root_spec(class, &op)) {
                        AdmissionDecision::Admitted { permit_id } => held.push(permit_id),
                        AdmissionDecision::Queued { ticket } => pending.push(ticket),
                        AdmissionDecision::Rejected(_) => {}
                    }
                }
                1 => {
                    if !held.is_empty() {
                        let p = held.swap_remove(rng.gen_range(0..held.len()));
                        g.release(p);
                        drain_promotions(g, &mut pending, &mut held);
                    }
                }
                2 => {
                    if !pending.is_empty() {
                        let t = pending.swap_remove(rng.gen_range(0..pending.len()));
                        g.abandon(t);
                    }
                }
                3 => drain_promotions(g, &mut pending, &mut held),
                4 => {
                    if !held.is_empty() {
                        let p = held[rng.gen_range(0..held.len())];
                        g.reconcile_memory(p, rng.gen_range(0..2_048));
                    }
                }
                5 => {
                    if !held.is_empty() {
                        let p = held[rng.gen_range(0..held.len())];
                        g.release_stage_memory(p, rng.gen_range(0..3));
                    }
                }
                _ => unreachable!(),
            }

            // --- hard caps must hold after every single op ---
            let snap = g.snapshot();
            assert!(
                cpu_held(&snap) <= CPU_BUDGET,
                "seed={seed}: cpu budget breached: {} > {CPU_BUDGET}",
                cpu_held(&snap)
            );
            for &c in &classes {
                let cs = snap.classes.get(&TaskClass::new(c.to_string())).unwrap();
                assert!(
                    cs.inflight <= max_inflight(c),
                    "seed={seed}: {c} inflight {} > cap {}",
                    cs.inflight,
                    max_inflight(c)
                );
                assert!(
                    cs.queued as usize <= max_depth(c),
                    "seed={seed}: {c} queue {} > depth {}",
                    cs.queued,
                    max_depth(c)
                );
            }
        }

        // --- full drain: nothing may be left stuck, leaked, or unaccounted ---
        let mut guard = 0;
        loop {
            drain_promotions(g, &mut pending, &mut held);
            if held.is_empty() && pending.is_empty() {
                break;
            }
            if let Some(t) = pending.pop() {
                g.abandon(t);
            } else if let Some(p) = held.pop() {
                g.release(p);
            }
            guard += 1;
            assert!(guard < 1_000_000, "seed={seed}: drain did not converge");
        }

        let snap = g.snapshot();
        assert_eq!(cpu_held(&snap), 0, "seed={seed}: cpu leaked: {snap:?}");
        assert_eq!(mem_held(&snap), 0, "seed={seed}: memory leaked: {snap:?}");
        for &c in &classes {
            let cs = snap.classes.get(&TaskClass::new(c.to_string())).unwrap();
            assert_eq!(cs.inflight, 0, "seed={seed}: {c} inflight leaked: {cs:?}");
            assert_eq!(cs.queued, 0, "seed={seed}: {c} queue leaked: {cs:?}");
        }
    }
}

// ===========================================================================
// 2. Exact capacity boundaries — no off-by-one in fail-closed rejection.
// ===========================================================================

#[test]
fn inflight_cap_is_exact_and_fails_closed() {
    // Non-queueable class: the (N+1)-th admit must reject, never squeeze in.
    let n = 5u32;
    let fx = fixture(
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(n)
                .overflow_policy(OverflowPolicy::Reject)
                .retry_after_policy(RetryAfterPolicy::FixedMs(7)),
        )],
        0,
        0,
    );
    let g = &*fx.governor;

    let mut permits = Vec::new();
    for i in 0..n {
        match g.admit(&root_spec("c", &format!("a{i}"))) {
            AdmissionDecision::Admitted { permit_id } => permits.push(permit_id),
            o => panic!("admit {i} within cap must succeed, got {o:?}"),
        }
        assert_eq!(inflight(&g.snapshot(), "c"), i + 1);
    }
    // Boundary: one over the cap rejects (fail-closed), never admits.
    match g.admit(&root_spec("c", "over")) {
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { retry_after_ms }) => {
            assert_eq!(retry_after_ms, Some(7), "fixed retry-after must surface");
        }
        o => panic!("over-cap admit must be CpuSaturated reject, got {o:?}"),
    }
    assert_eq!(
        inflight(&g.snapshot(), "c"),
        n,
        "cap must never be exceeded"
    );

    // Free exactly one slot → exactly one more admits.
    g.release(permits.pop().unwrap());
    assert_eq!(inflight(&g.snapshot(), "c"), n - 1);
    match g.admit(&root_spec("c", "refill")) {
        AdmissionDecision::Admitted { .. } => {}
        o => panic!("after release one slot must reopen, got {o:?}"),
    }
    assert_eq!(inflight(&g.snapshot(), "c"), n);
}

#[test]
fn global_cpu_budget_is_a_hard_ceiling_across_classes() {
    // Two classes, cost 1 cpu each, huge per-class cap → the *global* budget is
    // the only thing that can stop admission. It must stop at exactly K.
    let k = 6u32;
    let pol = || {
        ClassPolicy::new()
            .max_inflight(1_000)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::Reject)
    };
    let fx = fixture(vec![("a", pol()), ("b", pol())], k, 0);
    let g = &*fx.governor;

    let mut held = Vec::new();
    let mut admitted = 0u32;
    for i in 0..(k + 10) {
        let class = if i % 2 == 0 { "a" } else { "b" };
        let op = format!("g{i}");
        match g.admit(&root_spec(class, &op)) {
            AdmissionDecision::Admitted { permit_id } => {
                held.push(permit_id);
                admitted += 1;
            }
            AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. }) => {}
            o => panic!("expected admit-or-cpu-saturated, got {o:?}"),
        }
        assert!(
            cpu_held(&g.snapshot()) <= k,
            "global cpu budget breached mid-stream"
        );
    }
    assert_eq!(admitted, k, "exactly the budget worth of permits may live");
    assert_eq!(cpu_held(&g.snapshot()), k);

    for p in held {
        g.release(p);
    }
    assert_eq!(
        cpu_held(&g.snapshot()),
        0,
        "budget fully reclaimed on drain"
    );
}

// ===========================================================================
// 3. Recursion guard — exact (root, stage) occupancy, no breadth leak.
// ===========================================================================

#[test]
fn recursion_guard_rejects_reentry_to_an_active_root_stage() {
    let fx = fixture(vec![("worker", ClassPolicy::new().max_inflight(16))], 0, 0);
    let g = &*fx.governor;
    let worker = TaskClass::new("worker".to_string());

    // Root occupies (R, "blocking").
    let root = TaskSpec::blocking(worker.clone()).operation("R");
    let rp = match g.admit(&root) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("root must admit, got {o:?}"),
    };

    // First child declaring parent_stage "blocking" admits and occupies
    // (R, "blocking") — the declared lineage point, independent of its substrate.
    let child = |op: &'static str| {
        TaskSpec::cpu(worker.clone())
            .child_of("R", TaskStage::new("blocking"))
            .operation(op)
    };
    let c1 = match g.admit(&child("c1")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("first child must admit, got {o:?}"),
    };

    // Second child re-entering the *same* (R, "blocking") is a recursive loop.
    match g.admit(&child("c2")) {
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission) => {}
        o => panic!("re-entry to active (root,stage) must be RecursiveAdmission, got {o:?}"),
    }

    // Releasing the first child frees (R, "blocking") → a fresh child admits again.
    g.release(c1);
    match g.admit(&child("c3")) {
        AdmissionDecision::Admitted { permit_id } => g.release(permit_id),
        o => panic!("after release the stage must reopen, got {o:?}"),
    }
    g.release(rp);
    assert_eq!(inflight(&g.snapshot(), "worker"), 0);
}

// ===========================================================================
// 4. Memory overcommit — every policy honored exactly.
// ===========================================================================

#[test]
fn memory_overcommit_reject_is_fail_closed() {
    let fx = fixture(
        vec![(
            "heavy",
            ClassPolicy::new()
                .max_inflight(100)
                .memory_units(10)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Reject),
        )],
        0,
        10, // exactly one heavy permit fits
    );
    let g = &*fx.governor;
    match g.admit(&root_spec("heavy", "h1")) {
        AdmissionDecision::Admitted { .. } => {}
        o => panic!("first heavy must fit, got {o:?}"),
    }
    match g.admit(&root_spec("heavy", "h2")) {
        AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. }) => {}
        o => panic!("overcommit must reject (fail-closed), got {o:?}"),
    }
    assert_eq!(
        mem_held(&g.snapshot()),
        10,
        "no overcommit may slip through"
    );
}

#[test]
fn memory_overcommit_queue_pins_at_depth() {
    let depth = 4u32;
    let fx = fixture(
        vec![(
            "heavy",
            ClassPolicy::new()
                .max_inflight(100)
                .max_queue_depth(depth)
                .memory_units(10)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
        )],
        0,
        10,
    );
    let g = &*fx.governor;
    // One fits; the next `depth` queue; the one after that is shed.
    match g.admit(&root_spec("heavy", "h0")) {
        AdmissionDecision::Admitted { .. } => {}
        o => panic!("first must admit, got {o:?}"),
    }
    for i in 0..depth {
        match g.admit(&root_spec("heavy", &format!("q{i}"))) {
            AdmissionDecision::Queued { .. } => {}
            o => panic!("overcommit within depth must queue, got {o:?}"),
        }
    }
    match g.admit(&root_spec("heavy", "shed")) {
        AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. }) => {}
        o => panic!("past depth must shed, got {o:?}"),
    }
    assert_eq!(
        g.snapshot().classes[&TaskClass::new("heavy".to_string())].queued,
        depth
    );
}

#[test]
fn memory_overcommit_degrades_to_fallback_class() {
    // heavy(10) overcommits → degrade to light(1), which fits in the headroom.
    let fx = fixture(
        vec![
            (
                "heavy",
                ClassPolicy::new()
                    .max_inflight(100)
                    .memory_units(10)
                    .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
                        fallback_class: TaskClass::new("light".to_string()),
                    }),
            ),
            (
                "light",
                ClassPolicy::new().max_inflight(100).memory_units(1),
            ),
        ],
        0,
        11, // 10 (heavy) + 1 (light) exactly
    );
    let g = &*fx.governor;
    match g.admit(&root_spec("heavy", "h1")) {
        AdmissionDecision::Admitted { .. } => {}
        o => panic!("first heavy must fit, got {o:?}"),
    }
    // Second heavy cannot fit → degrades and is admitted *as light*.
    match g.admit(&root_spec("heavy", "h2")) {
        AdmissionDecision::Admitted { .. } => {}
        o => panic!("overcommit must degrade-admit, got {o:?}"),
    }
    assert_eq!(inflight(&g.snapshot(), "heavy"), 1, "only one ran as heavy");
    assert_eq!(
        inflight(&g.snapshot(), "light"),
        1,
        "the rest degraded to light"
    );
    assert_eq!(
        mem_held(&g.snapshot()),
        11,
        "held = 10 + 1, exactly at budget"
    );
}

// ===========================================================================
// 5. Leak sweep — precise, idempotent, and a no-op on already-gone permits.
// ===========================================================================

#[test]
fn leak_sweep_is_precise_and_idempotent() {
    // The sweep only reclaims classes that opted into LeakDetecting. A second
    // class on the default policy must NEVER be force-reclaimed — even when it is
    // just as stale. That opt-in boundary is the sharp edge under test.
    let fx = fixture(
        vec![
            (
                "retrieval",
                retrieval_policy(8, 8).memory_release_policy(MemoryReleasePolicy::LeakDetecting),
            ),
            // default memory_release_policy (OnTaskCompletion): not sweepable.
            ("managed", retrieval_policy(8, 8)),
        ],
        0,
        0,
    );
    let g = &*fx.governor;

    fx.clock.set(0);
    let mut ids = Vec::new();
    for i in 0..3 {
        match g.admit(&root_spec("retrieval", &format!("a{i}"))) {
            AdmissionDecision::Admitted { permit_id } => ids.push(permit_id),
            o => panic!("admit must succeed, got {o:?}"),
        }
    }
    // A managed permit, left untouched (so it is just as stale as a0/a1) — the
    // sweep must leave it alone because its class did not opt in.
    let managed = match g.admit(&root_spec("managed", "m0")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("managed admit must succeed, got {o:?}"),
    };

    // Touch the third at t=50 so it survives a 100ms staleness window at t=101.
    fx.clock.set(50);
    assert!(
        g.reconcile_memory(ids[2], 0),
        "touch must update last_touched"
    );

    fx.clock.set(101);
    let report = g.reap_leaks_with(100);
    assert_eq!(
        report.reclaimed_permits, 2,
        "exactly the two stale LeakDetecting permits: {report:?}"
    );
    assert_eq!(report.suspected_leaks, 2);

    // Idempotent: a second sweep reclaims nothing.
    assert_eq!(g.reap_leaks_with(100).reclaimed_permits, 0);
    assert_eq!(
        inflight(&g.snapshot(), "retrieval"),
        1,
        "touched leaky survivor stays inflight"
    );
    // Opt-in boundary: the non-LeakDetecting class is never force-reclaimed.
    assert_eq!(
        inflight(&g.snapshot(), "managed"),
        1,
        "a stale non-LeakDetecting permit must survive the sweep"
    );

    // Releasing an already-reaped permit is a safe no-op (no underflow).
    g.release(ids[0]);
    assert_eq!(inflight(&g.snapshot(), "retrieval"), 1);

    // Everything releases cleanly to zero.
    g.release(ids[2]);
    g.release(managed);
    assert_eq!(inflight(&g.snapshot(), "retrieval"), 0);
    assert_eq!(inflight(&g.snapshot(), "managed"), 0);
}

// ===========================================================================
// 6. Deterministic reduce — malformed fan-out never ships.
// ===========================================================================

#[test]
fn malformed_fanout_rejects_well_formed_admits() {
    let fx = fixture(vec![("m", ClassPolicy::new().max_inflight(4))], 0, 0);
    let g = &*fx.governor;
    let m = TaskClass::new("m".to_string());

    // Fan-out stage without a reduce policy is unshippable.
    let bad = TaskSpec::blocking(m.clone())
        .operation("bad")
        .fan_out_stage(TaskStage::new("map"), SubstrateHint::SharedCpuExecutor);
    assert!(Governor::validate_reduce(&bad).is_err());
    match g.admit(&bad) {
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask) => {}
        o => panic!("malformed fan-out must reject as MalformedTask, got {o:?}"),
    }

    // The same fan-out *with* a complete reduce policy admits.
    let good = TaskSpec::blocking(m.clone())
        .operation("good")
        .reduce_stage(
            TaskStage::new("map"),
            SubstrateHint::SharedCpuExecutor,
            DeterministicReducePolicy::keyed("score"),
        );
    assert!(Governor::validate_reduce(&good).is_ok());
    match g.admit(&good) {
        AdmissionDecision::Admitted { permit_id } => g.release(permit_id),
        o => panic!("well-formed fan-out must admit, got {o:?}"),
    }
}

// ===========================================================================
// 7. Lifecycle ops are idempotent and safe under abuse.
// ===========================================================================

#[test]
fn lifecycle_ops_are_idempotent_and_safe() {
    let fx = fixture(vec![("c", retrieval_policy(2, 4))], 0, 0);
    let g = &*fx.governor;
    let c = TaskClass::new("c".to_string());

    let p = match g.admit(&root_spec("c", "x")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("admit must succeed, got {o:?}"),
    };
    g.release(p);
    g.release(p); // double release: must be a no-op, never underflow
    assert_eq!(inflight(&g.snapshot(), "c"), 0);
    assert_eq!(g.snapshot().classes[&c].cpu_units_held, 0);

    // Unknown handles are all safe.
    assert!(g.claim(999_999).is_none(), "claim of unknown ticket → None");
    g.abandon(999_999); // unknown ticket: no-op
    assert!(
        !g.reconcile_memory(999_999, 100),
        "reconcile unknown → false"
    );
    assert_eq!(
        g.release_stage_memory(999_999, 5),
        0,
        "stage-release unknown → 0"
    );
    assert_eq!(g.reap_leaks_with(0).reclaimed_permits, 0, "nothing to reap");
}
