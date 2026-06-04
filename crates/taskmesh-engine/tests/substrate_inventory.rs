//! T08: substrate inventory is an explicit, duplicate-checked SSOT exposed in
//! the snapshot.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov_with(substrates: Vec<SubstrateRecord>) -> Result<Governor, GovernorError> {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("c"), ClassPolicy::new());
    let policy = PolicySet::new(ResourceBudget::new(), classes).with_substrates(substrates)?;
    Ok(Governor::new(policy, Arc::new(ManualClock::new(0))))
}

#[test]
fn builtin_substrates_appear_in_snapshot() {
    let g = gov_with(builtin_records()).unwrap();
    let names: Vec<String> = g
        .snapshot()
        .substrates
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    for builtin in BUILTIN_SUBSTRATES {
        assert!(names.contains(&builtin.to_string()), "missing {builtin}");
    }
}

#[test]
fn duplicate_substrate_registration_rejects() {
    let mut records = builtin_records();
    records.push(SubstrateRecord::new(
        "cpu",
        SubstrateKind::CompetingExecution,
        Some("cpu"),
    ));
    assert!(gov_with(records).is_err());
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
    let code: Vec<String> = BUILTIN_SUBSTRATES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        fixture, code,
        "BUILTIN_SUBSTRATES drifted from allowlist fixture"
    );
}

#[test]
fn snapshot_substrates_are_name_ordered() {
    let g = gov_with(builtin_records()).unwrap();
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
