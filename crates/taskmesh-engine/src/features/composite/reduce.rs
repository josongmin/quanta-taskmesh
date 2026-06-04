//! Deterministic reduce enforcement (T06): a fan-out stage is never shippable
//! without a complete `DeterministicReducePolicy`.

use taskmesh_contract::{DeterministicReducePolicy, GovernorError, TaskSpec};

/// A reduce policy is complete iff its stable sort key is non-empty. The
/// remaining fields are non-optional in the type, so presence is completeness.
pub fn validate_policy(policy: &DeterministicReducePolicy) -> Result<(), GovernorError> {
    if policy.stable_sort_key.trim().is_empty() {
        return Err(GovernorError::PolicyViolation(
            "deterministic reduce requires a non-empty stable_sort_key".into(),
        ));
    }
    Ok(())
}

/// Every fan-out stage in the spec must carry a complete reduce policy.
pub fn validate_spec(spec: &TaskSpec) -> Result<(), GovernorError> {
    for stage in &spec.stages {
        if stage.fan_out {
            match &stage.reduce_policy {
                None => {
                    return Err(GovernorError::PolicyViolation(
                        format!(
                            "parallel stage {} requires a deterministic reduce policy",
                            stage.stage
                        )
                        .into(),
                    ));
                }
                Some(policy) => validate_policy(policy)?,
            }
        }
    }
    Ok(())
}
