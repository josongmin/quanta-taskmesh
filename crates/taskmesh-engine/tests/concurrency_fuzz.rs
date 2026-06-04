//! Brutal multi-threaded, all-operations fuzz over the real `Governor`.
//!
//! Many OS threads concurrently interleave EVERY mutating governance op —
//! admit / release / memory-reconcile / stage-boundary-release — across classes
//! with heterogeneous policies (Estimated / Measured / Hybrid memory, different
//! caps and costs). The oracle is threefold and scheduling-independent, so the
//! test is hard but not flaky:
//!
//!   1. `GovernedState::assert_consistent` (debug) runs inside the engine on
//!      mutating ops and trips immediately if per-class/global/per-permit
//!      accounting ever diverges under any interleaving.
//!   2. Every permit id ever granted is globally unique (no double-grant).
//!   3. Per-class inflight never exceeds the cap, and after every owner releases
//!      its permits the whole governor drains to exactly zero.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::thread;

use taskmesh_contract::{
    ClassPolicy, ManualClock, MemoryPermitMode, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, RequestKey};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

/// (name, cap, cpu, mem, mode)
const SPECS: [(&str, u32, u32, u32, MemoryPermitMode); 3] = [
    ("est", 4, 1, 2, MemoryPermitMode::Estimated),
    ("meas", 8, 2, 1, MemoryPermitMode::Measured),
    ("hyb", 2, 1, 3, MemoryPermitMode::Hybrid),
];

fn governor() -> Arc<Governor> {
    let classes: BTreeMap<TaskClass, ClassPolicy> = SPECS
        .iter()
        .map(|(n, cap, cpu, mem, mode)| {
            (
                TaskClass::new(*n),
                ClassPolicy::new()
                    .max_inflight(*cap)
                    .cpu_units(*cpu)
                    .memory_units(*mem)
                    .memory_permit_mode(*mode),
            )
        })
        .collect();
    // Generous global budget + a real byte scale (required for measured/hybrid).
    let resources = ResourceBudget::new()
        .cpu_units(1_000_000)
        .memory_units(1_000_000)
        .memory_unit_scale(8);
    let policy = PolicySet::new(resources, classes);
    Governor::validate_policy(&policy).expect("policy valid");
    Arc::new(Governor::new(policy, Arc::new(ManualClock::new(0))))
}

#[test]
fn all_ops_concurrent_fuzz_stays_consistent_and_drains() {
    const THREADS: usize = 12;
    const OPS: usize = 3_000;
    let g = governor();

    let per_thread: Vec<Vec<u64>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|tid| {
                let g = Arc::clone(&g);
                scope.spawn(move || {
                    let mut lcg = Lcg(0x9E37_79B9_7F4A_7C15 ^ tid as u64);
                    // permits this thread currently holds: (id, class_idx)
                    let mut held: Vec<(u64, usize)> = Vec::new();
                    // every id this thread was ever granted (uniqueness proof)
                    let mut granted_ids: Vec<u64> = Vec::new();

                    for step in 0..OPS {
                        let roll = lcg.next() % 8;
                        match roll {
                            // admit (5/8 of the time — keep pressure high)
                            0..=4 => {
                                let ci = (lcg.next() as usize) % SPECS.len();
                                let op = format!("{tid}-{step}");
                                let spec = TaskSpec::blocking(TaskClass::new(SPECS[ci].0))
                                    .operation(op.clone());
                                match g.admit(&spec, RequestKey::new(op)) {
                                    AdmissionDecision::Admitted { permit_id } => {
                                        held.push((permit_id, ci));
                                        granted_ids.push(permit_id);
                                    }
                                    AdmissionDecision::Rejected(_) => {} // saturated: fine
                                    AdmissionDecision::Queued { .. } => {
                                        unreachable!("no queue configured")
                                    }
                                }
                            }
                            // release a random held permit
                            5 => {
                                if !held.is_empty() {
                                    let i = (lcg.next() as usize) % held.len();
                                    let (id, _) = held.swap_remove(i);
                                    g.release(id);
                                }
                            }
                            // reconcile measured memory on a held permit
                            6 => {
                                if !held.is_empty() {
                                    let i = (lcg.next() as usize) % held.len();
                                    let bytes = lcg.next() % 256;
                                    g.reconcile_memory(held[i].0, bytes);
                                }
                            }
                            // stage-boundary partial memory release on a held permit
                            _ => {
                                if !held.is_empty() {
                                    let i = (lcg.next() as usize) % held.len();
                                    let units = (lcg.next() % 4) as u32;
                                    g.release_stage_memory(held[i].0, units);
                                }
                            }
                        }

                        // Sample the cap invariant periodically (lock-consistent read).
                        if step % 64 == 0 {
                            let snap = g.snapshot();
                            for (name, cap, _, _, _) in SPECS {
                                let inflight = snap.classes[&TaskClass::new(name)].inflight;
                                assert!(
                                    inflight <= cap,
                                    "class {name} inflight {inflight} exceeded cap {cap}"
                                );
                            }
                        }
                    }

                    // Drain everything this thread still holds.
                    for (id, _) in held {
                        g.release(id);
                    }
                    granted_ids
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // No id was ever granted twice across all threads.
    let mut seen = BTreeSet::new();
    let mut total = 0usize;
    for ids in &per_thread {
        for &id in ids {
            assert!(seen.insert(id), "permit id {id} double-granted");
            total += 1;
        }
    }
    assert!(total > 0, "the fuzz must have granted some permits");

    // Everything released → the governor is exactly zero on every axis.
    let snap = g.snapshot();
    for (name, _, _, _, _) in SPECS {
        let c = &snap.classes[&TaskClass::new(name)];
        assert_eq!(
            (c.inflight, c.queued, c.cpu_units_held, c.memory_units_held),
            (0, 0, 0, 0),
            "class {name} did not drain clean: {c:?}"
        );
    }
}
