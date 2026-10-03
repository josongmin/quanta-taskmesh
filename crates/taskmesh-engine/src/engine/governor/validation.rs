//! Policy and inventory validation.

use super::{
    inventory, violation, FairnessPolicy, Governor, GovernorError, MemoryOvercommitPolicy,
    MemoryPermitMode, OverflowPolicy, PolicySet, SubstrateKind,
};

impl Governor {
    // ---- validation (T02) -------------------------------------------------

    /// Fail-closed configuration validation. Every impossible budget is rejected
    /// at construction, never deferred to a runtime admit path.
    pub fn validate_policy(policy: &PolicySet) -> Result<(), GovernorError> {
        Self::validate_inventory(policy)?;
        let budget = &policy.resources;
        for (class, class_policy) in &policy.classes {
            class
                .validate()
                .map_err(|error| violation(format!("invalid policy class {class:?}: {error}")))?;
            let cost = class_policy.permit_cost;

            if budget.per_request_max_cpu_units != 0
                && cost.cpu_units > budget.per_request_max_cpu_units
            {
                return Err(violation(format!(
                    "class {class} exceeds per-request cpu limit"
                )));
            }
            if budget.per_request_max_memory_units != 0
                && cost.memory_units > budget.per_request_max_memory_units
            {
                return Err(violation(format!(
                    "class {class} exceeds per-request memory limit"
                )));
            }
            if budget.max_cpu_units != 0 && cost.cpu_units > budget.max_cpu_units {
                return Err(violation(format!(
                    "class {class} exceeds global cpu budget"
                )));
            }
            if budget.max_memory_units != 0 && cost.memory_units > budget.max_memory_units {
                return Err(violation(format!(
                    "class {class} exceeds global memory budget"
                )));
            }

            if matches!(
                class_policy.memory_permit_mode,
                MemoryPermitMode::Measured | MemoryPermitMode::Hybrid
            ) && budget.memory_unit_scale.bytes_per_unit == 0
            {
                return Err(violation(format!(
                    "class {class}: measured/hybrid memory requires bytes_per_unit > 0"
                )));
            }

            if class_policy.overflow_policy == OverflowPolicy::QueueWithinDepth
                && class_policy.max_queue_depth == 0
            {
                return Err(violation(format!(
                    "class {class} is queueable but has max_queue_depth == 0"
                )));
            }

            // The scavenger discipline only dispatches in the best-effort tier;
            // declaring it on a non-best-effort class is contradictory.
            if class_policy.fairness == FairnessPolicy::BestEffortScavenger
                && !class_policy.best_effort
            {
                return Err(violation(format!(
                    "class {class} uses BestEffortScavenger fairness but is not best_effort"
                )));
            }

            // A zero weight has no proportional meaning. Coercing it to 1 would
            // make an obviously wrong config look like a deliberate one.
            if let FairnessPolicy::WeightedFairQueue { weight, .. } = class_policy.fairness {
                if weight == 0 {
                    return Err(violation(format!(
                        "class {class} declares WeightedFairQueue weight 0; weight must be >= 1"
                    )));
                }
            }

            if class_policy.memory_overcommit_policy == MemoryOvercommitPolicy::Queue
                && class_policy.max_queue_depth == 0
            {
                return Err(violation(format!(
                    "class {class} queues on overcommit but has max_queue_depth == 0"
                )));
            }

            if let MemoryOvercommitPolicy::DegradeToLight { fallback_class } =
                &class_policy.memory_overcommit_policy
            {
                if fallback_class == class {
                    return Err(violation(format!("class {class} degrades to itself")));
                }
                let Some(fallback) = policy.classes.get(fallback_class) else {
                    return Err(violation(format!(
                        "class {class} degrades to unknown fallback {fallback_class}"
                    )));
                };
                // A degrade is a promise of *some* service under memory
                // pressure. Pointing it at a class that can never dispatch turns
                // that promise into a rejection with a misleading verdict.
                if fallback.is_disabled() {
                    return Err(violation(format!(
                        "class {class} degrades to disabled fallback {fallback_class}"
                    )));
                }
                // A degrade is one hop (D03): a request that already degraded
                // is admitted against the fallback's resource account without
                // consulting the fallback's own overcommit policy. A fallback
                // that itself declares `DegradeToLight` therefore promises a
                // second hop the engine never takes — and on a cycle (a → b →
                // a) the "second hop" is the class that just failed. Either
                // way the configuration says something the runtime does not
                // do, so it is refused at construction.
                if matches!(
                    fallback.memory_overcommit_policy,
                    MemoryOvercommitPolicy::DegradeToLight { .. }
                ) {
                    return Err(violation(format!(
                        "class {class} degrades to fallback {fallback_class}, which itself \
                         degrades; a degrade is one hop and cannot chain"
                    )));
                }
            }
        }

        // Fairness is a per-tier discipline: when several classes are runnable in
        // the same tier, the scheduler arbitrates them with ONE discipline. A
        // heterogeneous mix would let the lexically-first class silently impose
        // its discipline on the others, so reject it at construction. Weights/
        // quanta/slack may still differ between classes (same discipline kind);
        // only the discipline *kind* must agree within a tier. Best-effort forms
        // a separate tier. Disabled classes never dispatch and are exempt.
        let mut primary_kind: Option<std::mem::Discriminant<FairnessPolicy>> = None;
        let mut best_effort_kind: Option<std::mem::Discriminant<FairnessPolicy>> = None;
        for (class, class_policy) in &policy.classes {
            if class_policy.is_disabled() {
                continue;
            }
            let kind = std::mem::discriminant(&class_policy.fairness);
            let tier = if class_policy.best_effort {
                &mut best_effort_kind
            } else {
                &mut primary_kind
            };
            match tier {
                None => *tier = Some(kind),
                Some(existing) if *existing != kind => {
                    return Err(violation(format!(
                        "class {class} mixes a different fairness discipline within its \
                         scheduling tier; all classes in a tier must share one discipline"
                    )));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Validate the substrate registry as a whole.
    ///
    /// Per-record validation at registration time is not enough: a registry can
    /// still be handed over empty, keyed under a name that disagrees with the
    /// record it holds, or missing a built-in the host's dispatch table assumes
    /// exists. The built-ins are intrinsic to *any* governor, so their presence
    /// and their exact kind/pool binding are checked here, where the policy
    /// becomes live, regardless of how the registry was assembled.
    fn validate_inventory(policy: &PolicySet) -> Result<(), GovernorError> {
        let registry = policy.substrates();
        for (key, record) in registry {
            inventory::validate_record(record)?;
            if key.as_str() != record.name.as_ref() {
                return Err(violation(format!(
                    "substrate registry key {key} does not match record name {}",
                    record.name
                )));
            }
        }
        for expected in inventory::builtin_records() {
            let Some(found) = registry.get(expected.name.as_ref()) else {
                return Err(violation(format!(
                    "substrate registry is missing built-in {}",
                    expected.name
                )));
            };
            if found.kind != expected.kind || found.capability_pool != expected.capability_pool {
                return Err(violation(format!(
                    "built-in substrate {} is registered with a non-canonical kind/pool binding",
                    expected.name
                )));
            }
        }
        for capability in policy.capability_records() {
            let bound = registry
                .values()
                .any(|record| inventory::provides_capability(record, capability.name()));
            if !bound {
                return Err(violation(format!(
                    "capability authority declared for pool {}, which no executing substrate provides",
                    capability.name()
                )));
            }
        }
        for record in registry.values() {
            if record.kind == SubstrateKind::AuthorityOnly {
                continue;
            }
            let Some(pool) = record.capability_pool.as_deref() else {
                continue;
            };
            if policy.resolve_capability(pool).is_err() {
                return Err(violation(format!(
                    "executing substrate {} has no capability authority for pool {pool}",
                    record.name
                )));
            }
        }
        Ok(())
    }
}
