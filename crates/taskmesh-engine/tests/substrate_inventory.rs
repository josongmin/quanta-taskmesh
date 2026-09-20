//! T08: substrate inventory is an explicit, duplicate-checked SSOT exposed in
//! the snapshot.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

// `PolicySet::new` auto-seeds the canonical built-ins, so callers pass only the
// *additional* deployment-specific substrates here.
fn gov_with(extra: Vec<SubstrateRecord>) -> Result<Governor, GovernorError> {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("c"), ClassPolicy::new());
    let policy = PolicySet::new(ResourceBudget::new(), classes).with_substrates(extra)?;
    Ok(Governor::new_unchecked(
        policy,
        Arc::new(ManualClock::new(0)),
    ))
}

#[test]
fn builtin_substrates_appear_in_snapshot() {
    // No extras: the built-ins are seeded by `PolicySet::new` itself.
    let g = gov_with(vec![]).unwrap();
    let names: Vec<String> = g
        .snapshot()
        .substrates
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    for builtin in BUILTIN_SUBSTRATES {
        assert!(names.contains(&(*builtin).to_string()), "missing {builtin}");
    }
}

#[test]
fn duplicate_substrate_registration_rejects() {
    // Re-registering a built-in name ("cpu") as a deployment extra must be
    // rejected — additions can never shadow the canonical inventory.
    let dup = SubstrateRecord::new("cpu", SubstrateKind::CompetingExecution, Some("cpu"));
    assert!(gov_with(vec![dup]).is_err());
}

#[test]
fn missing_capability_pool_rejects_unless_authority_only() {
    // CompetingExecution without a pool is invalid.
    let bad = SubstrateRecord::new("rogue", SubstrateKind::CompetingExecution, None::<&str>);
    assert!(gov_with(vec![bad]).is_err());

    // AuthorityOnly is allowed without a pool.
    let ok = SubstrateRecord::new("authority", SubstrateKind::AuthorityOnly, None::<&str>);
    assert!(gov_with(vec![ok]).is_ok());
}

#[test]
fn builtin_set_matches_allowlist_fixture() {
    // Drift detector: the canonical built-in set is the repo-native SSOT and must
    // stay in lockstep with the allowlist fixture.
    let raw = include_str!("fixtures/substrate_allowlist.json");
    let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
    let fixture: Vec<String> = parsed["builtins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let code: Vec<String> = BUILTIN_SUBSTRATES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        fixture, code,
        "BUILTIN_SUBSTRATES drifted from allowlist fixture"
    );
}

#[test]
fn snapshot_substrates_are_name_ordered() {
    let g = gov_with(vec![]).unwrap();
    let names: Vec<String> = g
        .snapshot()
        .substrates
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn a_substrate_with_an_empty_name_is_rejected() {
    // A record keyed on "" (or whitespace) could never be addressed by a
    // `TaskSpec` and would still count as an executing substrate for pool
    // binding. Both the empty and the whitespace spelling are refused by name.
    for name in ["", "   "] {
        let bad = SubstrateRecord::new(name, SubstrateKind::CompetingExecution, Some("blocking"));
        let Err(GovernorError::PolicyViolation(message)) = gov_with(vec![bad]) else {
            panic!("a substrate named {name:?} must be refused");
        };
        assert!(
            message.contains("substrate name must be non-empty"),
            "the refusal must name the rule, got: {message}"
        );
    }
}

#[test]
fn capability_resolution_is_typed_and_state_neutral_for_empty_unknown_and_typo_names() {
    let g = gov_with(vec![]).expect("valid built-ins");
    let before = g.snapshot();
    assert_eq!(
        g.policy().resolve_capability(""),
        Err(CapabilityResolutionError::EmptyName)
    );
    for name in ["missing", "blockng"] {
        assert_eq!(
            g.policy().resolve_capability(name),
            Err(CapabilityResolutionError::UnknownName {
                name: name.to_owned(),
            })
        );
    }
    assert_eq!(g.snapshot(), before);
    assert!(g.permit_ledgers().is_empty());
}

#[test]
fn a_registered_id_from_another_policy_is_rejected_before_admission() {
    let mut source_classes = BTreeMap::new();
    source_classes.insert(TaskClass::new("c"), ClassPolicy::new());
    let source_policy = PolicySet::new(ResourceBudget::new(), source_classes)
        .with_substrates(vec![SubstrateRecord::new(
            "custom-executor",
            SubstrateKind::CompetingExecution,
            Some("custom-pool"),
        )])
        .expect("valid custom substrate")
        .with_capability_limits(BTreeMap::from([("custom-pool".to_owned(), 1)]))
        .expect("custom capability has an executing substrate");
    let source =
        Governor::new(source_policy, Arc::new(ManualClock::new(0))).expect("source governor");
    let target = gov_with(vec![]).expect("target governor");
    let foreign = source
        .policy()
        .resolve_capability("custom-pool")
        .expect("source custom capability");
    let before = target.snapshot();
    assert_eq!(
        target.admit_resolved(
            &TaskSpec::blocking(TaskClass::new("c")).operation("foreign"),
            foreign,
            None,
        ),
        Err(CapabilityResolutionError::ForeignId)
    );
    assert_eq!(target.snapshot(), before);
    assert!(target.permit_ledgers().is_empty());
}

#[test]
fn same_name_with_a_different_policy_authority_is_foreign() {
    let policy = |limit| {
        let mut classes = BTreeMap::new();
        classes.insert(TaskClass::new("c"), ClassPolicy::new());
        PolicySet::new(ResourceBudget::new(), classes)
            .with_capability_limits(BTreeMap::from([("blocking".to_owned(), limit)]))
            .expect("built-in capability")
    };
    let source_policy = policy(1);
    let source_id = source_policy
        .resolve_capability("blocking")
        .expect("source authority");
    let target = Governor::new(policy(2), Arc::new(ManualClock::new(0))).expect("target governor");
    let before = target.snapshot();
    assert_eq!(
        target.admit_resolved(
            &TaskSpec::blocking(TaskClass::new("c")).operation("foreign-policy"),
            source_id,
            None,
        ),
        Err(CapabilityResolutionError::ForeignId)
    );
    assert_eq!(target.snapshot(), before);
}

#[test]
fn executing_substrate_without_capacity_authority_fails_closed() {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("c"), ClassPolicy::new());
    let policy = PolicySet::new(ResourceBudget::new(), classes)
        .with_substrates(vec![SubstrateRecord::new(
            "custom-executor",
            SubstrateKind::CompetingExecution,
            Some("custom-pool"),
        )])
        .expect("record shape is valid but authority is absent");
    let Err(GovernorError::PolicyViolation(message)) =
        Governor::new(policy, Arc::new(ManualClock::new(0)))
    else {
        panic!("missing capability authority must reject governor construction");
    };
    assert!(message.contains("custom-pool"));
    assert!(message.contains("no capability authority"));
}

#[test]
fn snapshot_and_admission_share_the_same_capacity_record() {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(4)
            .max_queue_depth(4)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let policy = PolicySet::new(ResourceBudget::new(), classes)
        .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 1)]))
        .expect("built-in authority");
    let capability = policy
        .resolve_capability("blocking")
        .expect("registered capability");
    let g = Governor::new(policy, Arc::new(ManualClock::new(0))).expect("valid policy");
    let spec = TaskSpec::blocking(TaskClass::new("c")).operation("one");
    let AdmissionDecision::Admitted { permit_id } = g
        .admit_resolved(&spec, capability.clone(), None)
        .expect("local resolved id")
    else {
        panic!("the one registered slot is free");
    };
    assert_eq!(g.snapshot().capabilities["blocking"].limit, 1);
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 1);
    assert!(matches!(
        g.admit_resolved(&spec.clone().operation("two"), capability, None)
            .expect("same registered id"),
        AdmissionDecision::Queued { .. }
    ));
    assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
}
