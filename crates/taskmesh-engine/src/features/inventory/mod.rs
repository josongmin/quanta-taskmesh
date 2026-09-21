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

/// Validate a record in isolation: the name must be non-empty, and every kind
/// except `AuthorityOnly` must name a **non-empty** capability pool. A `Some("")`
/// pool is as fail-open as `None`, so both are rejected.
pub fn validate_record(record: &SubstrateRecord) -> Result<(), GovernorError> {
    if record.name.trim().is_empty() {
        return Err(GovernorError::PolicyViolation(
            "substrate name must be non-empty".into(),
        ));
    }
    if record.kind != SubstrateKind::AuthorityOnly {
        let pool_ok = record
            .capability_pool
            .as_ref()
            .is_some_and(|p| !p.trim().is_empty());
        if !pool_ok {
            return Err(GovernorError::PolicyViolation(
                format!(
                    "substrate {} requires a non-empty capability pool",
                    record.name
                )
                .into(),
            ));
        }
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

/// Whether this record executes work against the named capability authority.
/// Authority-only records document a pool but never provide execution capacity.
pub fn provides_capability(record: &SubstrateRecord, capability: &str) -> bool {
    match record.kind {
        SubstrateKind::AuthorityOnly => false,
        _ => record.capability_pool.as_deref() == Some(capability),
    }
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

#[cfg(test)]
mod tests {
    use super::provides_capability;
    use taskmesh_contract::{SubstrateKind, SubstrateRecord};

    #[test]
    fn capability_provider_requires_both_execution_and_exact_pool_identity() {
        let executing =
            SubstrateRecord::new("gpu-runner", SubstrateKind::CompetingExecution, Some("gpu"));
        let authority =
            SubstrateRecord::new("gpu-authority", SubstrateKind::AuthorityOnly, Some("gpu"));

        assert!(provides_capability(&executing, "gpu"));
        assert!(!provides_capability(&executing, "cpu"));
        assert!(!provides_capability(&authority, "gpu"));
    }
}
