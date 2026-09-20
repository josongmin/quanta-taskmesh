use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, HeldCapacity, ManualClock, MemoryOvercommitPolicy,
    OverflowPolicy, PermitWaker, ResourceBudget, SubstrateHint, TaskClass, TaskSpec, TaskStage,
};
use taskmesh_engine::{
    AdmissionDecision, BlockerSet, CapacityAssessment, ClaimOutcome, Governor, PolicySet,
    ReleaseOutcome, TerminalReason,
};

fn class(name: &str) -> TaskClass {
    TaskClass::new(name.to_owned())
}

fn queueing(max_inflight: u32, cpu: u32, memory: u32) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(max_inflight)
        .max_queue_depth(16)
        .cpu_units(cpu)
        .memory_units(memory)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .memory_overcommit_policy(MemoryOvercommitPolicy::Queue)
}

fn governor(
    classes: impl IntoIterator<Item = (&'static str, ClassPolicy)>,
    cpu: u32,
    memory: u32,
    blocking: u32,
    large_stack: u32,
) -> Governor {
    let classes = classes
        .into_iter()
        .map(|(name, policy)| (class(name), policy))
        .collect();
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(cpu).memory_units(memory),
        classes,
    )
    .with_capability_limits(BTreeMap::from([
        ("blocking".to_owned(), blocking),
        ("large_stack".to_owned(), large_stack),
    ]))
    .expect("built-in capability authority");
    Governor::new(policy, Arc::new(ManualClock::new(10))).expect("valid policy")
}

fn rename_primary(mut spec: TaskSpec, stage: &str) -> TaskSpec {
    spec.stages[0].stage = TaskStage::new(stage.to_owned());
    spec
}

fn descendant_parent(spec: TaskSpec) -> TaskSpec {
    rename_primary(
        spec.child_of("root", "root", TaskStage::new("root-stage"))
            .operation("child-a"),
        "parent-stage",
    )
}

fn grandchild(spec: TaskSpec) -> TaskSpec {
    spec.awaited_child_of("root", "child-a", TaskStage::new("parent-stage"))
        .operation("grandchild")
}

fn admitted(g: &Governor, spec: &TaskSpec) -> u64 {
    match g.admit(spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn assert_cycle(decision: AdmissionDecision, expected: HeldCapacity) {
    assert_eq!(
        decision,
        AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle {
            held_by_root: expected,
        })
    );
}

#[test]
fn descendant_immediate_parent_capacity_is_detected_for_every_dimension() {
    let cases = ["class", "capability", "cpu", "memory"];
    for case in cases {
        let (g, parent_spec, child_spec, expected) = match case {
            "class" => (
                governor([("c", queueing(1, 1, 1))], 100, 100, 0, 0),
                descendant_parent(TaskSpec::io(class("c"))),
                grandchild(TaskSpec::io(class("c"))),
                HeldCapacity::ClassInflight { class: class("c") },
            ),
            "capability" => (
                governor(
                    [("p", queueing(4, 1, 1)), ("c", queueing(4, 1, 1))],
                    100,
                    100,
                    1,
                    0,
                ),
                descendant_parent(TaskSpec::blocking(class("p"))),
                grandchild(TaskSpec::blocking(class("c"))),
                HeldCapacity::CapabilityPool {
                    pool: "blocking".to_owned(),
                },
            ),
            "cpu" => (
                governor(
                    [("p", queueing(4, 2, 1)), ("c", queueing(4, 1, 1))],
                    2,
                    100,
                    0,
                    0,
                ),
                descendant_parent(TaskSpec::io(class("p"))),
                grandchild(TaskSpec::io(class("c"))),
                HeldCapacity::CpuBudget,
            ),
            "memory" => (
                governor(
                    [("p", queueing(4, 1, 2)), ("c", queueing(4, 1, 1))],
                    100,
                    2,
                    0,
                    0,
                ),
                descendant_parent(TaskSpec::io(class("p"))),
                grandchild(TaskSpec::io(class("c"))),
                HeldCapacity::MemoryBudget,
            ),
            _ => unreachable!(),
        };
        let parent = admitted(&g, &parent_spec);
        let before = g.snapshot();
        assert_cycle(g.admit(&child_spec), expected);
        assert_eq!(g.snapshot(), before, "{case}: rejection is state-neutral");
        assert_eq!(g.release(parent), ReleaseOutcome::Released);
    }
}

#[test]
fn a_parent_owned_blocker_is_not_hidden_by_an_earlier_stranger_blocker() {
    let g = governor(
        [("parent", queueing(4, 1, 1)), ("child", queueing(1, 1, 1))],
        100,
        100,
        1,
        0,
    );
    let parent = admitted(&g, &descendant_parent(TaskSpec::blocking(class("parent"))));
    let stranger = admitted(&g, &TaskSpec::io(class("child")).operation("stranger"));
    let child = grandchild(TaskSpec::blocking(class("child")));

    assert_cycle(
        g.admit(&child),
        HeldCapacity::CapabilityPool {
            pool: "blocking".to_owned(),
        },
    );
    assert_eq!(g.snapshot().classes[&class("child")].queued, 0);
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn sibling_capacity_is_reversible_and_does_not_form_a_false_cycle() {
    let g = governor(
        [("parent", queueing(4, 1, 1)), ("c", queueing(4, 1, 1))],
        100,
        100,
        1,
        0,
    );
    let parent = admitted(
        &g,
        &rename_primary(
            TaskSpec::io(class("parent")).operation("parent"),
            "parent-stage",
        ),
    );
    let sibling = admitted(
        &g,
        &rename_primary(
            TaskSpec::blocking(class("c"))
                .child_of("parent", "parent", TaskStage::new("sibling-stage"))
                .operation("sibling"),
            "sibling-stage",
        ),
    );
    let awaited = TaskSpec::blocking(class("c"))
        .awaited_child_of("parent", "parent", TaskStage::new("parent-stage"))
        .operation("awaited");
    let AdmissionDecision::Queued { ticket } = g.admit(&awaited) else {
        panic!("a sibling can finish independently, so this is a reversible wait");
    };
    assert!(matches!(
        g.pending_assessment(ticket),
        Some(CapacityAssessment::ReversiblyBlocked(_))
    ));
    assert_eq!(g.release(sibling), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("releasing the sibling promotes the awaited child");
    };
    assert_eq!(g.release(child), ReleaseOutcome::Released);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
}

#[test]
fn same_parent_operation_text_under_another_root_is_not_parent_evidence() {
    let g = governor(
        [("parent", queueing(4, 1, 1)), ("c", queueing(4, 1, 1))],
        100,
        100,
        1,
        0,
    );
    let stranger = admitted(
        &g,
        &rename_primary(
            TaskSpec::blocking(class("parent"))
                .child_of("root-a", "root-a", TaskStage::new("root-a-stage"))
                .operation("shared-parent-name"),
            "holder-stage",
        ),
    );
    let request = TaskSpec::blocking(class("c"))
        .awaited_child_of(
            "root-b",
            "shared-parent-name",
            TaskStage::new("root-b-parent-stage"),
        )
        .operation("child");
    let AdmissionDecision::Queued { ticket } = g.admit(&request) else {
        panic!("an operation under another root is a reversible stranger holder");
    };
    assert!(matches!(
        g.pending_assessment(ticket),
        Some(CapacityAssessment::ReversiblyBlocked(_))
    ));
    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(child) = g.claim(ticket) else {
        panic!("the child is promoted when the stranger releases");
    };
    assert_eq!(g.release(child), ReleaseOutcome::Released);
}

#[test]
fn duplicate_active_operation_identity_within_one_root_fails_closed() {
    let g = governor([("c", queueing(8, 1, 1))], 100, 100, 0, 0);
    let first = admitted(
        &g,
        &TaskSpec::io(class("c"))
            .child_of("root", "parent", TaskStage::new("stage-a"))
            .operation("duplicate-operation"),
    );
    let before = g.snapshot();
    let duplicate = TaskSpec::io(class("c"))
        .child_of("root", "parent", TaskStage::new("stage-b"))
        .operation("duplicate-operation");
    assert_eq!(
        g.admit(&duplicate),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    );
    assert_eq!(g.snapshot(), before);
    assert_eq!(g.permit_ledgers().len(), 1);
    assert_eq!(g.release(first), ReleaseOutcome::Released);

    let reused = admitted(&g, &duplicate);
    assert_eq!(g.release(reused), ReleaseOutcome::Released);
}

struct CountWake(Arc<AtomicUsize>);

impl PermitWaker for CountWake {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn promotion_reassessment_terminalizes_a_newly_formed_parent_cycle() {
    let g = governor(
        [("parent", queueing(4, 1, 1)), ("child", queueing(1, 1, 1))],
        100,
        100,
        1,
        0,
    );
    let stranger = admitted(&g, &TaskSpec::io(class("child")).operation("stranger"));
    let woke = Arc::new(AtomicUsize::new(0));
    let waker: Arc<dyn PermitWaker> = Arc::new(CountWake(Arc::clone(&woke)));
    let queued = TaskSpec::blocking(class("child"))
        .awaited_child_of("root", "parent", TaskStage::new("parent-stage"))
        .operation("queued-child");
    let AdmissionDecision::Queued { ticket } = g.admit_waitable(&queued, waker) else {
        panic!("the unrelated class holder initially creates a reversible wait");
    };
    let parent = admitted(
        &g,
        &rename_primary(
            TaskSpec::blocking(class("parent"))
                .child_of("root", "root", TaskStage::new("root-stage"))
                .operation("parent"),
            "parent-stage",
        ),
    );

    assert_eq!(g.release(stranger), ReleaseOutcome::Released);
    assert_eq!(woke.load(Ordering::SeqCst), 1);
    assert_eq!(
        g.claim(ticket),
        ClaimOutcome::Terminal(TerminalReason::IrreversibleWaitCycle {
            held_by_parent: HeldCapacity::CapabilityPool {
                pool: "blocking".to_owned(),
            },
        })
    );
    assert_eq!(g.snapshot().classes[&class("child")].queued, 0);
    assert_eq!(g.release(parent), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn pending_diagnostics_recompute_the_full_current_blocker_set() {
    let g = governor(
        [("holder", queueing(4, 1, 1)), ("c", queueing(1, 1, 1))],
        100,
        100,
        1,
        0,
    );
    let capability_holder = admitted(
        &g,
        &TaskSpec::blocking(class("holder")).operation("cap-holder"),
    );
    let class_holder = admitted(&g, &TaskSpec::io(class("c")).operation("class-holder"));
    let AdmissionDecision::Queued { ticket } =
        g.admit(&TaskSpec::blocking(class("c")).operation("pending"))
    else {
        panic!("the request has simultaneous class and capability blockers");
    };

    let Some(CapacityAssessment::ReversiblyBlocked(BlockerSet {
        class_inflight: true,
        capabilities,
        ..
    })) = g.pending_assessment(ticket)
    else {
        panic!("current assessment must expose all simultaneous blockers");
    };
    assert_eq!(capabilities.len(), 1);

    assert_eq!(g.release(capability_holder), ReleaseOutcome::Released);
    let Some(CapacityAssessment::ReversiblyBlocked(blockers)) = g.pending_assessment(ticket) else {
        panic!("the class blocker remains");
    };
    assert!(blockers.class_inflight);
    assert!(blockers.capabilities.is_empty());

    assert_eq!(g.release(class_holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
        panic!("clearing the last current blocker promotes the request");
    };
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

#[test]
fn multiple_capability_requirements_charge_and_release_atomically() {
    let g = governor([("c", queueing(8, 1, 1))], 100, 100, 1, 1);
    let plan = TaskSpec::blocking(class("c"))
        .operation("multi-capability")
        .stage(
            TaskStage::new("large-stack-stage"),
            SubstrateHint::LargeStackCapability,
        );
    let first = admitted(&g, &plan);
    let snapshot = g.snapshot();
    assert_eq!(snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(snapshot.capabilities["large_stack"].in_use, 1);
    assert_eq!(
        g.permit_ledger(first)
            .expect("live permit")
            .capabilities
            .iter()
            .count(),
        2
    );

    let AdmissionDecision::Queued { ticket } =
        g.admit(&plan.clone().operation("multi-capability-2"))
    else {
        panic!("both required pools are occupied by the first permit");
    };
    assert_eq!(g.release(first), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(second) = g.claim(ticket) else {
        panic!("both requirements become available in the same transition");
    };
    let promoted = g.snapshot();
    assert_eq!(promoted.capabilities["blocking"].in_use, 1);
    assert_eq!(promoted.capabilities["large_stack"].in_use, 1);
    assert_eq!(g.release(second), ReleaseOutcome::Released);
    let final_snapshot = g.snapshot();
    assert_eq!(final_snapshot.capabilities["blocking"].in_use, 0);
    assert_eq!(final_snapshot.capabilities["large_stack"].in_use, 0);
    assert_eq!(final_snapshot.conservation_violation(), None);
}

#[test]
fn invalid_duplicate_stage_is_rejected_before_ids_or_state_change() {
    let g = governor([("c", queueing(8, 1, 1))], 100, 100, 1, 1);
    let invalid = TaskSpec::io(class("c"))
        .operation("duplicate-stage")
        .stage(TaskStage::new("io"), SubstrateHint::BlockingPool);
    let before = g.snapshot();
    assert_eq!(
        g.admit(&invalid),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    );
    assert_eq!(g.snapshot(), before);
    assert_eq!(g.permit_ledgers().len(), 0);
    assert_eq!(g.retained_terminal_tickets(), 0);

    let permit = admitted(&g, &TaskSpec::io(class("c")).operation("valid"));
    assert_eq!(permit, 1, "invalid input did not consume a permit id");
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}
