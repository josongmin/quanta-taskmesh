//! Regression: `TokioRuntime::config()` is a faithful, serializable description
//! of the built runtime — it carries the resolved substrate inventory (built-ins
//! plus deployment additions), not a reduced view that only the governor
//! snapshot can reconstruct.

use taskmesh::ext::*;
use taskmesh::*;

fn names(records: &[SubstrateRecord]) -> Vec<String> {
    records.iter().map(|s| s.name.to_string()).collect()
}

#[test]
fn config_carries_resolved_substrate_inventory() {
    let extra = SubstrateRecord::new(
        "external-gpu",
        SubstrateKind::CompetingExecution,
        Some("external-gpu"),
    );
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .substrate(extra)
        .build()
        .unwrap();

    let config_names = names(&rt.config().substrates);

    // Every built-in is present in the config inventory.
    for builtin in BUILTIN_SUBSTRATES {
        assert!(
            config_names.contains(&(*builtin).to_string()),
            "config inventory missing built-in {builtin}"
        );
    }
    // The deployment addition is present too.
    assert!(config_names.contains(&"external-gpu".to_string()));

    // And the config inventory matches the governor snapshot exactly (same
    // authority, two views): config() is not lossy.
    let snapshot_names = names(&rt.governor().snapshot().substrates);
    assert_eq!(
        config_names, snapshot_names,
        "config inventory must mirror the governor snapshot inventory"
    );
}
