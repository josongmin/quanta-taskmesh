//! TM16-004 regression: the substrate registry is validated state, and no
//! public path produces a governor whose inventory disagrees with the built-in
//! set the host's dispatch table assumes exists.
//!
//! The original defect had two halves. `PolicySet::default()` built an *empty*
//! registry while the canonical `new()` seeded the built-ins, so which
//! constructor you happened to call decided whether the runtime had an
//! inventory at all. And the registry was a public field, so a caller could
//! `clear()` it, or insert a record under a key that disagreed with its own
//! name, after every per-record check had already run.
//!
//! The fix is structural — `Default` delegates to the canonical constructor and
//! the field is private — but structure alone is not a proof, so the validator
//! is also exercised directly against registries the safe API can no longer
//! build.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, GovernorError, ManualClock, ResourceBudget, SubstrateKind, SubstrateRecord,
    TaskClass,
};
use taskmesh_engine::{builtin_records, Governor, PolicySet, BUILTIN_SUBSTRATES};

fn classes() -> BTreeMap<TaskClass, ClassPolicy> {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new().max_inflight(4).cpu_units(1),
    );
    classes
}

fn canonical_registry() -> BTreeMap<String, SubstrateRecord> {
    builtin_records()
        .into_iter()
        .map(|record| (record.name.to_string(), record))
        .collect()
}

/// Build a governor from a forged registry, reporting only the error. The
/// governor itself is not `Debug`, and the assertions here are about *why* a
/// registry was refused, so the success case collapses to a marker.
fn governor_from(registry: BTreeMap<String, SubstrateRecord>) -> Result<(), GovernorError> {
    Governor::new(
        PolicySet::new(ResourceBudget::new(), classes()).forge_substrates(registry),
        Arc::new(ManualClock::new(0)),
    )
    .map(|_| ())
}

#[test]
fn default_policy_set_carries_the_canonical_inventory() {
    // The whole point of the `Default` delegation: there is no constructor that
    // yields an inventory-less governor.
    let governor = Governor::new(PolicySet::default(), Arc::new(ManualClock::new(0)))
        .expect("the default policy set must be constructible");
    let names: Vec<String> = governor
        .substrates()
        .into_iter()
        .map(|record| record.name.to_string())
        .collect();
    let mut expected: Vec<String> = BUILTIN_SUBSTRATES.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    assert_eq!(names, expected);
}

#[test]
fn a_governor_cannot_boot_with_an_empty_inventory() {
    let error = governor_from(BTreeMap::new()).expect_err("empty inventory must be rejected");
    let GovernorError::PolicyViolation(message) = error else {
        panic!("expected a policy violation, got {error:?}");
    };
    assert!(
        message.contains("missing built-in"),
        "the message must name the missing built-in: {message}"
    );
}

#[test]
fn every_builtin_must_be_present() {
    for absent in BUILTIN_SUBSTRATES {
        let mut registry = canonical_registry();
        registry.remove(*absent);
        let error = governor_from(registry)
            .err()
            .unwrap_or_else(|| panic!("a missing built-in must be rejected: {absent}"));
        let GovernorError::PolicyViolation(message) = error else {
            panic!("expected a policy violation for {absent}, got {error:?}");
        };
        assert!(
            message.contains(absent),
            "the message must name {absent}: {message}"
        );
    }
}

#[test]
fn a_registry_key_that_disagrees_with_its_record_is_rejected() {
    // The shape a public field allowed: the record is individually valid, and
    // the map has the right number of entries, but a lookup by name finds
    // something else. Per-record validation cannot see this.
    let mut registry = canonical_registry();
    let record = registry.remove("cpu").expect("canonical cpu record");
    registry.insert("not-cpu".to_string(), record);
    let error = governor_from(registry).expect_err("key/record mismatch must be rejected");
    let GovernorError::PolicyViolation(message) = error else {
        panic!("expected a policy violation, got {error:?}");
    };
    assert!(
        message.contains("does not match") || message.contains("missing built-in"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_builtin_rebound_to_another_pool_is_rejected() {
    // Present, correctly keyed, individually valid — and pointing at the wrong
    // capability pool, so every gate derived from it would size the wrong thing.
    let mut registry = canonical_registry();
    registry.insert(
        "large_stack".to_string(),
        SubstrateRecord::new(
            "large_stack",
            SubstrateKind::CompetingExecution,
            Some("blocking"),
        ),
    );
    let error = governor_from(registry).expect_err("rebound built-in must be rejected");
    let GovernorError::PolicyViolation(message) = error else {
        panic!("expected a policy violation, got {error:?}");
    };
    assert!(
        message.contains("non-canonical"),
        "unexpected message: {message}"
    );
}

#[test]
fn a_builtin_with_the_wrong_kind_is_rejected() {
    let mut registry = canonical_registry();
    registry.insert(
        "cpu".to_string(),
        SubstrateRecord::new("cpu", SubstrateKind::AuthorityOnly, Some("cpu")),
    );
    let error = governor_from(registry).expect_err("rekinded built-in must be rejected");
    assert!(matches!(error, GovernorError::PolicyViolation(_)));
}

#[test]
fn the_canonical_registry_is_accepted() {
    // Positive control: the negative cases above fail for the reason claimed,
    // not because this validator rejects everything.
    governor_from(canonical_registry()).expect("the canonical registry must be accepted");
}

#[test]
fn extra_substrates_stay_tracking_only() {
    // Deployment additions are inventory, not run targets: they appear in the
    // snapshot and do not displace or shadow a built-in.
    let policy = PolicySet::new(ResourceBudget::new(), classes())
        .with_substrates(vec![SubstrateRecord::new(
            "external-gpu",
            SubstrateKind::CompetingExecution,
            Some("external-gpu"),
        )])
        .expect("an additional substrate registers");
    let governor =
        Governor::new(policy, Arc::new(ManualClock::new(0))).expect("still a valid policy");
    let names: Vec<String> = governor
        .substrates()
        .into_iter()
        .map(|record| record.name.to_string())
        .collect();
    assert!(names.contains(&"external-gpu".to_string()));
    for builtin in BUILTIN_SUBSTRATES {
        assert!(names.contains(&(*builtin).to_owned()), "lost {builtin}");
    }
}

#[test]
fn shadowing_a_builtin_is_rejected() {
    let error = PolicySet::new(ResourceBudget::new(), classes())
        .with_substrates(vec![SubstrateRecord::new(
            "cpu",
            SubstrateKind::CompetingExecution,
            Some("cpu"),
        )])
        .expect_err("a built-in cannot be re-registered");
    assert!(matches!(error, GovernorError::PolicyViolation(_)));
}

#[test]
fn a_capability_limit_for_an_unregistered_pool_is_rejected() {
    // A limit nothing consults is a silent misconfiguration: the operator
    // believes a pool is bounded and it is not.
    let error = PolicySet::new(ResourceBudget::new(), classes())
        .with_capability_limits(BTreeMap::from([("ghost-pool".to_string(), 4)]))
        .expect_err("a limit must name a registered pool");
    let GovernorError::PolicyViolation(message) = error else {
        panic!("expected a policy violation, got {error:?}");
    };
    assert!(message.contains("ghost-pool"), "unexpected: {message}");
}
