//! Substrate inventory (T08): an explicit, duplicate-checked registry of
//! capability pools. The built-in canonical set is fixed; `local_runtime` is a
//! real registered substrate, not a catch-all escape hatch.

use std::collections::BTreeMap;

use taskmesh_contract::{GovernorError, SubstrateKind, SubstrateRecord};

/// The fixed canonical built-in substrates. This list is the repo-native SSOT.
pub const BUILTIN_SUBSTRATES: &[&str] = &[
    "cpu",
    "blocking",
    "large_stack",
    "maintenance",
    "local_runtime",
];

// ---- domain (pure) --------------------------------------------------------

/// Validate a record in isolation: every kind except `AuthorityOnly` must name a
/// capability pool, and the name must be non-empty.
pub fn validate_record(record: &SubstrateRecord) -> Result<(), GovernorError> {
    if record.name.is_empty() {
        return Err(GovernorError::PolicyViolation(
            "substrate name must be non-empty".into(),
        ));
    }
    if record.kind != SubstrateKind::AuthorityOnly && record.capability_pool.is_none() {
        return Err(GovernorError::PolicyViolation(
            format!("substrate {} requires a capability pool", record.name).into(),
        ));
    }
    Ok(())
}

/// The canonical built-in records, each bound to its own capability pool.
pub fn builtin_records() -> Vec<SubstrateRecord> {
    vec![
        SubstrateRecord::new("cpu", SubstrateKind::CompetingExecution, Some("cpu")),
        SubstrateRecord::new(
            "blocking",
            SubstrateKind::CompetingExecution,
            Some("blocking"),
        ),
        SubstrateRecord::new(
            "large_stack",
            SubstrateKind::CompetingExecution,
            Some("large_stack"),
        ),
        SubstrateRecord::new(
            "maintenance",
            SubstrateKind::MaintenanceOnly,
            Some("maintenance"),
        ),
        SubstrateRecord::new(
            "local_runtime",
            SubstrateKind::CompetingExecution,
            Some("local_runtime"),
        ),
    ]
}

// ---- service (stateful) ---------------------------------------------------

/// Register a substrate, rejecting duplicates and invalid records.
pub fn register(
    registry: &mut BTreeMap<String, SubstrateRecord>,
    record: SubstrateRecord,
) -> Result<(), GovernorError> {
    validate_record(&record)?;
    let name = record.name.to_string();
    if registry.contains_key(&name) {
        return Err(GovernorError::PolicyViolation(
            format!("duplicate substrate registration: {name}").into(),
        ));
    }
    registry.insert(name, record);
    Ok(())
}

/// Deterministic snapshot view (ordered by name).
pub fn snapshot(registry: &BTreeMap<String, SubstrateRecord>) -> Vec<SubstrateRecord> {
    registry.values().cloned().collect()
}
