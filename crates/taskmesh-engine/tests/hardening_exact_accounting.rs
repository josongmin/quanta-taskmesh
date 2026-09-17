//! TM16-008 regression: capacity is decided with exact arithmetic, and the
//! ledger is wide enough to hold what it grants.
//!
//! The original defect was `saturating_add(cost) > budget`. At the top of the
//! `u32` domain the sum saturates *to* `u32::MAX`, which is not greater than a
//! `u32::MAX` budget — so two requests of `2^31` each were both admitted
//! against a budget of `2^32 - 1`. The saturation then hid the overdraft from
//! the consistency check as well, because the recorded total saturated too.
//!
//! Every assertion below compares against an independent `u128` oracle computed
//! from the per-permit ledgers, not against the engine's own running totals: a
//! conservation check that reuses the arithmetic it is checking proves nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, MemoryPermitMode, ResourceBudget, TaskClass,
    TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, PolicySet, ReleaseOutcome};

fn gov(budget: ResourceBudget, classes: Vec<(&str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = classes
        .into_iter()
        .map(|(name, policy)| (TaskClass::new(name.to_string()), policy))
        .collect();
    Governor::new(
        PolicySet::new(budget, classes),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

fn spec(class: &str, op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new(class.to_string())).operation(op.to_string())
}

fn try_admit(g: &Governor, class: &str, op: &str) -> AdmissionDecision {
    g.admit(&spec(class, op))
}

fn admit(g: &Governor, class: &str, op: &str) -> PermitId {
    match try_admit(g, class, op) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

/// Independent oracle: recompute the aggregates from the per-permit ledgers in
/// exact `u128` and compare with what the snapshot reports.
fn assert_oracle_agrees(g: &Governor) {
    let mut cpu: u128 = 0;
    let mut memory: u128 = 0;
    let mut per_class: BTreeMap<TaskClass, (u128, u128, u32)> = BTreeMap::new();
    for ledger in g.permit_ledgers() {
        cpu += u128::from(ledger.cpu_units);
        memory += u128::from(ledger.effective_units);
        let entry = per_class.entry(ledger.class.clone()).or_default();
        entry.0 += u128::from(ledger.cpu_units);
        entry.1 += u128::from(ledger.effective_units);
        entry.2 += 1;
    }

    let snapshot = g.snapshot();
    assert_eq!(snapshot.conservation_violation(), None);
    let mut snapshot_cpu: u128 = 0;
    let mut snapshot_memory: u128 = 0;
    for (class, observed) in &snapshot.classes {
        snapshot_cpu += observed.cpu_units_held;
        snapshot_memory += observed.memory_units_held;
        let (expected_cpu, expected_memory, expected_inflight) =
            per_class.get(class).copied().unwrap_or_default();
        assert_eq!(
            observed.cpu_units_held, expected_cpu,
            "class {class} cpu disagrees with the ledger oracle"
        );
        assert_eq!(
            observed.memory_units_held, expected_memory,
            "class {class} memory disagrees with the ledger oracle"
        );
        assert_eq!(
            observed.inflight, expected_inflight,
            "class {class} inflight disagrees with the ledger oracle"
        );
    }
    assert_eq!(snapshot_cpu, cpu, "global cpu disagrees with the oracle");
    assert_eq!(
        snapshot_memory, memory,
        "global memory disagrees with the oracle"
    );
}

#[test]
fn two_half_max_cpu_requests_cannot_both_fit_a_max_budget() {
    // 2^31 + 2^31 = 2^32, which does not fit a budget of 2^32 - 1.
    const HALF: u32 = 1 << 31;
    let g = gov(
        ResourceBudget::new().cpu_units(u32::MAX),
        vec![("c", ClassPolicy::new().max_inflight(8).cpu_units(HALF))],
    );

    let first = admit(&g, "c", "first");
    assert_oracle_agrees(&g);

    let second = try_admit(&g, "c", "second");
    assert!(
        matches!(
            second,
            AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
        ),
        "the second request overruns the budget and must be rejected, got {second:?}"
    );

    // The exact total is recorded, not a saturated stand-in.
    assert_eq!(
        g.snapshot().classes[&TaskClass::new("c")].cpu_units_held,
        u128::from(HALF)
    );
    assert_eq!(g.release(first), ReleaseOutcome::Released);
    assert_oracle_agrees(&g);
}

#[test]
fn two_half_max_memory_requests_cannot_both_fit_a_max_budget() {
    const HALF: u32 = 1 << 31;
    let g = gov(
        ResourceBudget::new().memory_units(u32::MAX),
        vec![("c", ClassPolicy::new().max_inflight(8).memory_units(HALF))],
    );
    let first = admit(&g, "c", "first");
    let second = try_admit(&g, "c", "second");
    assert!(
        matches!(
            second,
            AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
        ),
        "got {second:?}"
    );
    assert_oracle_agrees(&g);
    assert_eq!(g.release(first), ReleaseOutcome::Released);
    assert_oracle_agrees(&g);
}

#[test]
fn the_budget_boundary_is_exact_on_both_sides() {
    // `MAX - 1` fits; the request that would make it `MAX + 1` does not. An
    // off-by-one here is the difference between a budget and a suggestion.
    let g = gov(
        ResourceBudget::new().cpu_units(u32::MAX),
        vec![
            (
                "big",
                ClassPolicy::new().max_inflight(8).cpu_units(u32::MAX - 1),
            ),
            ("one", ClassPolicy::new().max_inflight(8).cpu_units(1)),
            ("two", ClassPolicy::new().max_inflight(8).cpu_units(2)),
        ],
    );
    let big = admit(&g, "big", "big");
    // Exactly reaches the budget.
    let one = admit(&g, "one", "one");
    assert_eq!(
        g.snapshot()
            .classes
            .values()
            .map(|c| c.cpu_units_held)
            .sum::<u128>(),
        u128::from(u32::MAX)
    );
    // One unit more does not fit.
    assert!(matches!(
        try_admit(&g, "one", "over"),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
    assert_oracle_agrees(&g);
    assert_eq!(g.release(one), ReleaseOutcome::Released);
    // Now two units do not fit but one does.
    assert!(matches!(
        try_admit(&g, "two", "two"),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
    let one = admit(&g, "one", "again");
    assert_oracle_agrees(&g);
    assert_eq!(g.release(one), ReleaseOutcome::Released);
    assert_eq!(g.release(big), ReleaseOutcome::Released);
    assert_oracle_agrees(&g);
}

#[test]
fn an_unlimited_budget_still_accounts_exactly_past_the_u32_domain() {
    // `0` means "no limit", so the *aggregate* can legitimately exceed `u32::MAX`
    // even though each request's cost cannot. A `u32` total would wrap or
    // saturate here and start reporting less memory than is held.
    let g = gov(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(8)
                .cpu_units(u32::MAX)
                .memory_units(u32::MAX),
        )],
    );
    let mut held = Vec::new();
    for i in 0..5 {
        held.push(admit(&g, "c", &format!("op{i}")));
    }
    let observed = &g.snapshot().classes[&TaskClass::new("c")];
    assert_eq!(observed.cpu_units_held, u128::from(u32::MAX) * 5);
    assert_eq!(observed.memory_units_held, u128::from(u32::MAX) * 5);
    assert!(observed.cpu_units_held > u128::from(u32::MAX));
    assert_oracle_agrees(&g);

    for permit in held {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    let observed = &g.snapshot().classes[&TaskClass::new("c")];
    assert_eq!(observed.cpu_units_held, 0);
    assert_eq!(observed.memory_units_held, 0);
    assert_oracle_agrees(&g);
}

#[test]
fn release_permutations_return_exactly_what_was_taken() {
    // Large costs across several classes and roots, released in a rotated order:
    // the totals have to come back to zero regardless of the order they unwind.
    let g = gov(
        ResourceBudget::new(),
        vec![
            (
                "a",
                ClassPolicy::new()
                    .max_inflight(8)
                    .cpu_units(1 << 30)
                    .memory_units(7),
            ),
            (
                "b",
                ClassPolicy::new()
                    .max_inflight(8)
                    .cpu_units(3)
                    .memory_units(1 << 29),
            ),
        ],
    );
    for rotation in 0..4usize {
        let mut held = Vec::new();
        for i in 0..4 {
            held.push(admit(
                &g,
                if i % 2 == 0 { "a" } else { "b" },
                &format!("op{i}"),
            ));
        }
        assert_oracle_agrees(&g);
        held.rotate_left(rotation);
        for permit in held {
            assert_eq!(g.release(permit), ReleaseOutcome::Released);
            assert_oracle_agrees(&g);
        }
        for class in ["a", "b"] {
            let observed = &g.snapshot().classes[&TaskClass::new(class.to_string())];
            assert_eq!(observed.cpu_units_held, 0, "rotation {rotation}");
            assert_eq!(observed.memory_units_held, 0, "rotation {rotation}");
            assert_eq!(observed.inflight, 0, "rotation {rotation}");
        }
    }
}

#[test]
fn an_unconvertible_measurement_is_reported_not_silently_zeroed() {
    use taskmesh_contract::ResourceConversionError;
    use taskmesh_engine::ReconcileOutcome;

    // A `Measured` class with a 1 byte/unit scale cannot express a reading above
    // `u32::MAX` units. Converting it to a saturated (or zero) unit count would
    // under-report real memory, so the reconcile fails loudly instead.
    let g = gov(
        ResourceBudget::new().memory_unit_scale(1),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(4)
                .memory_units(4)
                .memory_permit_mode(MemoryPermitMode::Measured),
        )],
    );
    let permit = admit(&g, "c", "op");
    let before = g.snapshot().classes[&TaskClass::new("c")].memory_units_held;

    let outcome = g.reconcile_memory_at(permit, u64::from(u32::MAX) + 1, 1);
    assert_eq!(
        outcome,
        ReconcileOutcome::ConversionFailed(ResourceConversionError::MemoryUnitsOverflow {
            bytes: u64::from(u32::MAX) + 1,
            bytes_per_unit: 1,
        })
    );
    // A failed conversion leaves the ledger exactly as it was — no partial commit.
    assert_eq!(
        g.snapshot().classes[&TaskClass::new("c")].memory_units_held,
        before
    );
    assert_oracle_agrees(&g);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}
