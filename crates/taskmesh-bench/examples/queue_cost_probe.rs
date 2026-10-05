//! Diagnostic A02 queue-cost probe. Run the same source and release build against
//! two engine revisions; each row is one completed queue fill or promotion.
//! No wall-clock threshold or release qualification is inferred from a row.

use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::time::Instant;

use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, SubstrateKind, SubstrateRecord,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PolicySet, ReleaseOutcome, ResolvedCapability,
};

fn queue_policy(depth: usize) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(u32::try_from(depth).expect("queue depth fits u32"))
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
}

fn new_governor(depth: usize, disjoint_capabilities: bool) -> Governor {
    let mut classes = BTreeMap::from([
        (
            TaskClass::new("cpu-holder"),
            ClassPolicy::new().max_inflight(1).cpu_units(1),
        ),
        (TaskClass::new("queued"), queue_policy(depth)),
    ]);
    if disjoint_capabilities {
        classes.insert(
            TaskClass::new("cap-holder"),
            ClassPolicy::new().max_inflight(1),
        );
    }
    let mut policy = PolicySet::new(ResourceBudget::new().cpu_units(1), classes);
    if disjoint_capabilities {
        policy = policy
            .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 1)]))
            .expect("built-in blocking capability exists");
    }
    Governor::new(policy, Arc::new(ManualClock::new(0))).expect("production-valid policy")
}

fn cpu_holder(governor: &Governor) -> taskmesh_engine::PermitId {
    match governor.admit(&TaskSpec::local(TaskClass::new("cpu-holder")).operation("cpu-holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("CPU holder must admit: {other:?}"),
    }
}

fn fill(depth: usize) -> u128 {
    let governor = new_governor(depth, false);
    let _holder = cpu_holder(&governor);
    let specs: Vec<_> = (0..depth)
        .map(|index| {
            TaskSpec::blocking(TaskClass::new("queued")).operation(format!("fill-{index}"))
        })
        .collect();
    let start = Instant::now();
    for spec in &specs {
        assert!(matches!(
            governor.admit(spec),
            AdmissionDecision::Queued { .. }
        ));
    }
    let elapsed = start.elapsed().as_nanos();
    let snapshot = governor.snapshot();
    assert_eq!(
        snapshot.classes[&TaskClass::new("queued")].queued as usize,
        depth
    );
    assert!(snapshot.conservation_violation().is_none());
    elapsed
}

fn promotion(depth: usize) -> u128 {
    let governor = new_governor(depth, true);
    let cap_holder = match governor
        .admit(&TaskSpec::blocking(TaskClass::new("cap-holder")).operation("cap-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("capability holder must admit: {other:?}"),
    };
    let cpu_holder = cpu_holder(&governor);
    for index in 0..(depth - 1) {
        let spec =
            TaskSpec::blocking(TaskClass::new("queued")).operation(format!("blocked-{index}"));
        assert!(matches!(
            governor.admit(&spec),
            AdmissionDecision::Queued { .. }
        ));
    }
    let runnable = TaskSpec::cpu(TaskClass::new("queued")).operation("runnable-cpu");
    let ticket = match governor.admit(&runnable) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("runnable follower must initially queue behind CPU holder: {other:?}"),
    };
    let start = Instant::now();
    assert_eq!(governor.release(cpu_holder), ReleaseOutcome::Released);
    let elapsed = start.elapsed().as_nanos();
    assert!(matches!(governor.claim(ticket), ClaimOutcome::Ready(_)));
    let snapshot = governor.snapshot();
    assert_eq!(
        snapshot.classes[&TaskClass::new("queued")].queued as usize,
        depth - 1
    );
    assert!(snapshot.conservation_violation().is_none());
    // Keep the blocked capability occupied through the measured release.
    assert_eq!(governor.release(cap_holder), ReleaseOutcome::Released);
    elapsed
}

// Engine-only adversarial topology: one explicitly registered pool per queued
// request. Every request needs exactly one capability (the public per-request
// maximum is 32). These synthetic pools are not host workers and are not a
// recommended production inventory.
fn unique_capability_governor(depth: usize) -> (Governor, Vec<ResolvedCapability>) {
    let classes = BTreeMap::from([
        (
            TaskClass::new("cap-holder"),
            ClassPolicy::new().max_inflight(u32::try_from(depth).expect("bounded depth")),
        ),
        (
            TaskClass::new("cpu-holder"),
            ClassPolicy::new().max_inflight(1).cpu_units(1),
        ),
        (TaskClass::new("queued"), queue_policy(depth)),
    ]);
    let substrates: Vec<_> = (0..depth)
        .map(|index| {
            let name = format!("synthetic-cap-{index}");
            SubstrateRecord::new(name.clone(), SubstrateKind::CompetingExecution, Some(name))
        })
        .collect();
    let limits: BTreeMap<_, _> = (0..depth)
        .map(|index| (format!("synthetic-cap-{index}"), 1))
        .collect();
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(1), classes)
        .with_substrates(substrates)
        .expect("synthetic substrate inventory is explicit")
        .with_capability_limits(limits)
        .expect("every synthetic pool is registered");
    let capabilities = (0..depth)
        .map(|index| {
            policy
                .resolve_capability(&format!("synthetic-cap-{index}"))
                .expect("every synthetic capability has authority")
        })
        .collect();
    let governor =
        Governor::new(policy, Arc::new(ManualClock::new(0))).expect("production-valid policy");
    (governor, capabilities)
}

fn promotion_unique_capabilities(depth: usize) -> u128 {
    let (governor, capabilities) = unique_capability_governor(depth);
    for (index, capability) in capabilities.iter().take(depth - 1).enumerate() {
        let holder =
            TaskSpec::blocking(TaskClass::new("cap-holder")).operation(format!("held-cap-{index}"));
        assert!(matches!(
            governor.admit_resolved(&holder, capability.clone(), None),
            Ok(AdmissionDecision::Admitted { .. })
        ));
    }
    let cpu_holder = cpu_holder(&governor);
    for (index, capability) in capabilities.iter().take(depth - 1).enumerate() {
        let spec =
            TaskSpec::blocking(TaskClass::new("queued")).operation(format!("blocked-{index}"));
        assert!(matches!(
            governor.admit_resolved(&spec, capability.clone(), None),
            Ok(AdmissionDecision::Queued { .. })
        ));
    }
    let runnable = TaskSpec::blocking(TaskClass::new("queued")).operation("runnable-tail");
    let ticket = match governor.admit_resolved(&runnable, capabilities[depth - 1].clone(), None) {
        Ok(AdmissionDecision::Queued { ticket }) => ticket,
        other => panic!("tail request must queue behind CPU holder: {other:?}"),
    };
    let start = Instant::now();
    assert_eq!(governor.release(cpu_holder), ReleaseOutcome::Released);
    let elapsed = start.elapsed().as_nanos();
    assert!(matches!(governor.claim(ticket), ClaimOutcome::Ready(_)));
    let snapshot = governor.snapshot();
    assert_eq!(
        snapshot.classes[&TaskClass::new("queued")].queued as usize,
        depth - 1
    );
    assert!(snapshot.conservation_violation().is_none());
    elapsed
}

fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(
        args.len(),
        4,
        "usage: queue_cost_probe fill|promotion|promotion_unique_caps depth repeats"
    );
    let case = args[1].as_str();
    let depth: usize = args[2].parse().expect("numeric queue depth");
    let repeats: usize = args[3].parse().expect("numeric repetition count");
    assert!(depth >= 2 && repeats >= 1);
    for repetition in 0..repeats {
        let wall_ns = match case {
            "fill" => fill(depth),
            "promotion" => promotion(depth),
            "promotion_unique_caps" => promotion_unique_capabilities(depth),
            _ => panic!("unknown case: {case}"),
        };
        let completed = if case == "fill" { depth } else { 1 };
        println!("{case}\t{depth}\t{repetition}\t{wall_ns}\t{completed}");
    }
}
