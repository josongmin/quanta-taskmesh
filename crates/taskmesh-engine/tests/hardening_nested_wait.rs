//! Declared nested-wait cycles (H16-010-A06, ADR 0003 D12).
//!
//! A parent that awaits its child holds its permit until the child has run.
//! If the capacity the child needs is held *entirely* by that parent (the
//! root-scoped permits of the child's own root), no release can ever come.
//! Queueing the child would be a deadlock by declaration; the engine refuses
//! it before admission with `NestedWaitCycle`, naming the capacity, and
//! charges nothing.
//!
//! The rule is sound, not complete — it never refuses a wait that could end:
//!
//! * an *undeclared* child (`child_of`) queues as any request does — the wait
//!   graph inside an opaque closure is never inferred;
//! * capacity shared with a stranger (another root) is a wait, not a cycle;
//! * capacity shared with a *sibling* child is a wait, not a cycle — a sibling
//!   can finish without the parent;
//! * a class that can degrade to a fallback with room is admitted there.
//!
//! Every property is pinned by a mutant in `tools/verification/mutations.json`
//! (`nested-wait-*`).

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, HeldCapacity, ManualClock, MemoryOvercommitPolicy,
    OverflowPolicy, ResourceBudget, TaskClass, TaskSpec, TaskStage,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, PolicySet, ReleaseOutcome};

fn class(name: &'static str) -> TaskClass {
    TaskClass::new(name)
}

struct Shape {
    max_inflight: u32,
    cpu_units: u32,
    memory_units: u32,
    overflow: OverflowPolicy,
    overcommit: MemoryOvercommitPolicy,
}

impl Shape {
    fn queueing(max_inflight: u32) -> Self {
        Self {
            max_inflight,
            cpu_units: 1,
            memory_units: 1,
            overflow: OverflowPolicy::QueueWithinDepth,
            overcommit: MemoryOvercommitPolicy::Reject,
        }
    }
}

fn governor(
    shapes: Vec<(&'static str, Shape)>,
    cpu_budget: u32,
    memory_budget: u32,
    blocking_limit: u32,
) -> Governor {
    let mut classes = BTreeMap::new();
    for (name, shape) in shapes {
        classes.insert(
            class(name),
            ClassPolicy::new()
                .max_inflight(shape.max_inflight)
                .max_queue_depth(8)
                .cpu_units(shape.cpu_units)
                .memory_units(shape.memory_units)
                .overflow_policy(shape.overflow)
                .memory_overcommit_policy(shape.overcommit),
        );
    }
    let policy = PolicySet::new(
        ResourceBudget::new()
            .cpu_units(cpu_budget)
            .memory_units(memory_budget),
        classes,
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_string(), blocking_limit)]))
    .expect("blocking is a built-in pool");
    Governor::new(policy, Arc::new(ManualClock::new(1_000))).expect("valid policy")
}

fn root(class_name: &'static str, op: &str) -> TaskSpec {
    TaskSpec::io(class(class_name)).operation(op.to_string())
}

fn blocking_root(class_name: &'static str, op: &str) -> TaskSpec {
    TaskSpec::blocking(class(class_name)).operation(op.to_string())
}

fn awaited_child(class_name: &'static str, root_op: &str) -> TaskSpec {
    TaskSpec::io(class(class_name))
        .awaited_child_of(
            root_op.to_string(),
            root_op.to_string(),
            TaskStage::new("child"),
        )
        .operation("child")
}

fn undeclared_child(class_name: &'static str, root_op: &str) -> TaskSpec {
    TaskSpec::io(class(class_name))
        .child_of(
            root_op.to_string(),
            root_op.to_string(),
            TaskStage::new("child"),
        )
        .operation("child")
}

fn admit(g: &Governor, spec: &TaskSpec) -> PermitId {
    match g.admit(spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("{}: expected admission, got {other:?}", spec.operation),
    }
}

fn refused_as_cycle(what: &str, decision: AdmissionDecision, expected: &HeldCapacity) {
    match decision {
        AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle { held_by_root }) => {
            assert_eq!(
                &held_by_root, expected,
                "{what}: the verdict must name the capacity the root holds"
            );
        }
        other => panic!("{what}: a declared cycle must be refused as NestedWaitCycle, not queued or shed; got {other:?}"),
    }
}

fn queued(what: &str, decision: AdmissionDecision) -> u64 {
    match decision {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("{what}: a wait that can end must queue, got {other:?}"),
    }
}

// ---- the cycle, on each kind of capacity ----------------------------------

#[test]
fn an_awaited_child_blocked_only_by_its_own_root_is_refused_not_queued() {
    // One slot. The root holds it and awaits a child of the same class: the
    // child can only wait for the root, which waits for the child.
    let g = governor(vec![("c", Shape::queueing(1))], 100, 100, 0);
    let parent = admit(&g, &root("c", "parent"));
    let before = g.snapshot();

    refused_as_cycle(
        "same-class child",
        g.admit(&awaited_child("c", "parent")),
        &HeldCapacity::ClassInflight { class: class("c") },
    );
    let after = g.snapshot();
    assert_eq!(
        after.classes, before.classes,
        "a refused cycle must queue nothing and charge nothing"
    );
    assert_eq!(after.classes[&class("c")].queued, 0);
    assert_eq!(after.conservation_violation(), None);

    // The verdict is about this moment, not the child: once the root is gone
    // the same submission is admitted.
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
    let child = admit(&g, &awaited_child("c", "parent"));
    assert_eq!(g.release(child), ReleaseOutcome::Released);
}

#[test]
fn a_capability_pool_held_only_by_the_root_is_a_cycle_across_classes() {
    // The root (class p) holds the only blocking slot; the child (class q,
    // plenty of room) needs that pool. Different class, same pool: a cycle.
    let g = governor(
        vec![("p", Shape::queueing(4)), ("q", Shape::queueing(4))],
        100,
        100,
        1,
    );
    let parent = admit(&g, &blocking_root("p", "parent"));
    let child = TaskSpec::blocking(class("q"))
        .awaited_child_of(
            "parent".to_string(),
            "parent".to_string(),
            TaskStage::new("child"),
        )
        .operation("child");
    refused_as_cycle(
        "pool child",
        g.admit(&child),
        &HeldCapacity::CapabilityPool {
            pool: "blocking".to_string(),
        },
    );
    assert_eq!(g.snapshot().classes[&class("q")].queued, 0);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn a_cpu_budget_held_only_by_the_root_is_a_cycle() {
    // Budget 2 cpu units; the root costs 2. The child's class has room, the
    // pool is ungated: only the budget blocks it, and the root holds all of it.
    let heavy = Shape {
        cpu_units: 2,
        ..Shape::queueing(4)
    };
    let g = governor(vec![("h", heavy), ("c", Shape::queueing(4))], 2, 100, 0);
    let parent = admit(&g, &root("h", "parent"));
    refused_as_cycle(
        "cpu child",
        g.admit(&awaited_child("c", "parent")),
        &HeldCapacity::CpuBudget,
    );
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn a_memory_budget_held_only_by_the_root_is_a_cycle_whether_the_class_queues_or_sheds() {
    // Memory budget 2; the root costs 2. A class whose overcommit policy
    // *queues* would park the child forever; one that *sheds* would tell the
    // caller to retry forever. Both are the cycle.
    for overcommit in [
        MemoryOvercommitPolicy::Queue,
        MemoryOvercommitPolicy::Reject,
    ] {
        let heavy = Shape {
            memory_units: 2,
            ..Shape::queueing(4)
        };
        let child_class = Shape {
            overcommit: overcommit.clone(),
            ..Shape::queueing(4)
        };
        let g = governor(vec![("h", heavy), ("c", child_class)], 100, 2, 0);
        let parent = admit(&g, &root("h", "parent"));
        refused_as_cycle(
            &format!("memory child under {overcommit:?}"),
            g.admit(&awaited_child("c", "parent")),
            &HeldCapacity::MemoryBudget,
        );
        assert_eq!(g.snapshot().classes[&class("c")].queued, 0);
        assert_eq!(g.release(parent), ReleaseOutcome::Released);
    }
}

#[test]
fn a_shedding_class_reports_the_cycle_rather_than_a_retry_hint() {
    // A Reject-overflow class would shed with CpuSaturated + retry hint. The
    // hint would be a lie: the parent waits for this child, so no retry can
    // find the slot free. The verdict says so, and carries no hint.
    let shedding = Shape {
        overflow: OverflowPolicy::Reject,
        ..Shape::queueing(1)
    };
    let g = governor(vec![("c", shedding)], 100, 100, 0);
    let parent = admit(&g, &root("c", "parent"));
    let decision = g.admit(&awaited_child("c", "parent"));
    refused_as_cycle(
        "shed child",
        decision.clone(),
        &HeldCapacity::ClassInflight { class: class("c") },
    );
    let AdmissionDecision::Rejected(verdict) = decision else {
        unreachable!()
    };
    assert_eq!(
        verdict.retry_after_ms(),
        None,
        "a cycle has no useful retry"
    );
    assert!(
        verdict
            .to_string()
            .contains("declared nested wait cycle: class c inflight"),
        "the verdict renders the capacity: {verdict}"
    );
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

// ---- what is NOT a cycle ---------------------------------------------------

#[test]
fn an_undeclared_child_queues_as_any_request_does() {
    // Same shape as the first test, but the child does not declare that its
    // parent waits. Lineage is not a wait, and the engine infers none (D12):
    // the child queues and runs when the parent — which may well not be
    // waiting — finishes.
    let g = governor(vec![("c", Shape::queueing(1))], 100, 100, 0);
    let parent = admit(&g, &root("c", "parent"));
    let ticket = queued(
        "undeclared child",
        g.admit(&undeclared_child("c", "parent")),
    );
    assert_eq!(
        g.snapshot().classes[&class("c")].queued,
        1,
        "an undeclared wait must not be inferred as a cycle"
    );
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
    let taskmesh_engine::ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("the queued child is promoted once the parent releases")
    };
    assert_eq!(g.release(child), ReleaseOutcome::Released);
}

#[test]
fn capacity_shared_with_a_stranger_is_a_wait_not_a_cycle() {
    // Two slots: the root holds one, an unrelated root the other. The stranger
    // can finish, so the awaited child is queued — and runs when it does.
    let g = governor(vec![("c", Shape::queueing(2))], 100, 100, 0);
    let parent = admit(&g, &root("c", "parent"));
    let stranger = admit(&g, &root("c", "stranger"));
    let ticket = queued(
        "child behind a stranger",
        g.admit(&awaited_child("c", "parent")),
    );
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let taskmesh_engine::ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("the stranger's release promotes the child")
    };
    assert_eq!(g.snapshot().classes[&class("c")].inflight, 2);
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn a_pool_cpu_or_memory_budget_shared_with_a_stranger_is_a_wait_not_a_cycle() {
    // The class-inflight case above, on the three other kinds of capacity: the
    // root holds part of what the child waits for, a stranger the rest. The
    // stranger can release, so each child queues — and is promoted when it does.
    let claim = |g: &Governor, ticket: u64, what: &str| {
        let taskmesh_engine::ClaimOutcome::Ready(permit) = g.claim(ticket) else {
            panic!("{what}: the stranger's release promotes the child")
        };
        permit
    };

    // Capability pool: two blocking slots, one each.
    let g = governor(
        vec![("p", Shape::queueing(4)), ("q", Shape::queueing(4))],
        100,
        100,
        2,
    );
    let parent = admit(&g, &blocking_root("p", "parent"));
    let stranger = admit(&g, &blocking_root("p", "stranger"));
    let child = TaskSpec::blocking(class("q"))
        .awaited_child_of(
            "parent".to_string(),
            "parent".to_string(),
            TaskStage::new("child"),
        )
        .operation("child");
    let ticket = queued("pool shared with a stranger", g.admit(&child));
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let child = claim(&g, ticket, "pool");
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);

    // CPU budget 2: one unit each.
    let g = governor(
        vec![("h", Shape::queueing(4)), ("c", Shape::queueing(4))],
        2,
        100,
        0,
    );
    let parent = admit(&g, &root("h", "parent"));
    let stranger = admit(&g, &root("h", "stranger"));
    let ticket = queued(
        "cpu shared with a stranger",
        g.admit(&awaited_child("c", "parent")),
    );
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let child = claim(&g, ticket, "cpu");
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);

    // Memory budget 2: one unit each; the child's class queues on overcommit.
    let queue_on_memory = Shape {
        overcommit: MemoryOvercommitPolicy::Queue,
        ..Shape::queueing(4)
    };
    let g = governor(
        vec![("h", Shape::queueing(4)), ("c", queue_on_memory)],
        100,
        2,
        0,
    );
    let parent = admit(&g, &root("h", "parent"));
    let stranger = admit(&g, &root("h", "stranger"));
    let ticket = queued(
        "memory shared with a stranger",
        g.admit(&awaited_child("c", "parent")),
    );
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let child = claim(&g, ticket, "memory");
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn a_root_that_holds_none_of_the_pool_is_not_what_the_child_waits_for() {
    // The root is async I/O (no pool); a stranger holds the only blocking slot.
    // The child waits for the stranger, not for its parent: a wait. Counting
    // the root's permit toward a pool it does not occupy would call it a cycle.
    let g = governor(
        vec![("p", Shape::queueing(4)), ("q", Shape::queueing(4))],
        100,
        100,
        1,
    );
    let parent = admit(&g, &root("p", "parent"));
    let stranger = admit(&g, &blocking_root("p", "stranger"));
    let child = TaskSpec::blocking(class("q"))
        .awaited_child_of(
            "parent".to_string(),
            "parent".to_string(),
            TaskStage::new("child"),
        )
        .operation("child");
    let ticket = queued("child behind a stranger's pool slot", g.admit(&child));
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let taskmesh_engine::ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("the stranger's release promotes the child")
    };
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn capacity_shared_with_a_sibling_is_a_wait_not_a_cycle() {
    // Two slots: the root holds one, an earlier child of the same root the
    // other. Only root-scoped permits are "cannot end before this child": the
    // sibling is a child-scoped permit that finishes on its own, whether or
    // not the parent awaits it too.
    let g = governor(vec![("c", Shape::queueing(2))], 100, 100, 0);
    let parent = admit(&g, &root("c", "parent"));
    let sibling = admit(
        &g,
        &TaskSpec::io(class("c"))
            .awaited_child_of(
                "parent".to_string(),
                "parent".to_string(),
                TaskStage::new("first"),
            )
            .operation("sibling"),
    );
    let ticket = queued(
        "child behind a sibling",
        g.admit(&awaited_child("c", "parent")),
    );
    assert_eq!(g.release(sibling), ReleaseOutcome::Released);
    let taskmesh_engine::ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("the sibling's release promotes the child")
    };
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn a_class_that_degrades_to_a_fallback_with_room_is_admitted_not_refused() {
    // Memory budget 2, held by the root. The child's class degrades to a
    // light class that costs no memory: it is admitted there — the fallback
    // is judged before the cycle, because a fallback with room is not a wait.
    let heavy = Shape {
        memory_units: 2,
        ..Shape::queueing(4)
    };
    let degrading = Shape {
        overcommit: MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: class("light"),
        },
        ..Shape::queueing(4)
    };
    let light = Shape {
        memory_units: 0,
        ..Shape::queueing(4)
    };
    let g = governor(
        vec![("h", heavy), ("c", degrading), ("light", light)],
        100,
        2,
        0,
    );
    let parent = admit(&g, &root("h", "parent"));
    let child = admit(&g, &awaited_child("c", "parent"));
    assert_eq!(
        g.snapshot().classes[&class("light")].inflight,
        1,
        "the child runs in the fallback class"
    );
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}
