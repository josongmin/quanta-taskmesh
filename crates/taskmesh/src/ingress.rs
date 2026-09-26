//! Bounded JSON bytes ingress for untrusted task and runtime declarations.
//!
//! Raw contract DTO `Deserialize` remains a compatibility surface. This module
//! is the separate, fail-closed boundary for promoting bytes into authority.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use taskmesh_contract::{
    ClassPolicy, ClassificationRationale, PlanSource, ResourceBudget, StageDescriptor,
    SubstrateHint, SubstrateRecord, TaskClass, TaskPlanError, TaskScope, TaskSpec, TaskStage,
    TopologyConfig, ValidatedTaskPlan,
};

use crate::{Builder, MAX_REQUESTED_STACK_BYTES};

/// Conservative defaults for a governance declaration, not a payload body.
/// Integrators can choose lower limits for their deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrictIngressLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_stages: usize,
}

impl Default for StrictIngressLimits {
    fn default() -> Self {
        Self {
            max_bytes: 1 << 20,
            max_depth: 32,
            max_stages: 64,
        }
    }
}

/// A rejection before any Builder, Governor, worker, permit, or ticket exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StrictIngressError {
    InvalidLimit(&'static str),
    ByteLimit { actual: usize, max: usize },
    DepthLimit { max: usize },
    StageLimit { max: usize },
    DuplicateKey { key: String },
    UnknownKey { object: &'static str, key: String },
    InvalidShape(&'static str),
    Decode(String),
    TaskPlan(TaskPlanError),
    BlockingDispatch(&'static str),
}

impl fmt::Display for StrictIngressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit(name) => write!(f, "strict ingress limit {name} must be nonzero"),
            Self::ByteLimit { actual, max } => {
                write!(f, "strict ingress input is {actual} bytes, above {max}")
            }
            Self::DepthLimit { max } => write!(f, "strict ingress depth exceeds {max}"),
            Self::StageLimit { max } => write!(f, "strict ingress stages exceed {max}"),
            Self::DuplicateKey { key } => write!(f, "duplicate JSON key {key:?}"),
            Self::UnknownKey { object, key } => {
                write!(f, "unknown key {key:?} in {object}")
            }
            Self::InvalidShape(shape) => write!(f, "invalid JSON shape for {shape}"),
            Self::Decode(error) => write!(f, "invalid strict JSON: {error}"),
            Self::TaskPlan(error) => write!(f, "invalid task plan: {error}"),
            Self::BlockingDispatch(error) => write!(f, "invalid blocking dispatch: {error}"),
        }
    }
}

impl Error for StrictIngressError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Task,
    Scope,
    Child,
    Stages,
    Stage,
    Reduce,
    BlockingDispatch,
    RequestedStack,
    Config,
    Topology,
    Cpu,
    CpuMode,
    PhysicalDomains,
    PhysicalMode,
    Resources,
    MemoryScale,
    Classes,
    ClassPolicy,
    RetryAfter,
    PermitCost,
    Fairness,
    Wfq,
    Drr,
    DeadlineAware,
    Overcommit,
    Degrade,
    Checkpoint,
    Substrates,
    Substrate,
    CapabilityLimits,
    Scalar,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Scope => "scope",
            Self::Child => "child",
            Self::Stages => "stages",
            Self::Stage => "stage",
            Self::Reduce => "reduce_policy",
            Self::BlockingDispatch => "blocking_dispatch",
            Self::RequestedStack => "requested_stack",
            Self::Config => "runtime config",
            Self::Topology => "topology",
            Self::Cpu => "cpu topology",
            Self::CpuMode => "cpu mode",
            Self::PhysicalDomains => "physical_domains",
            Self::PhysicalMode => "physical domain mode",
            Self::Resources => "resources",
            Self::MemoryScale => "memory_unit_scale",
            Self::Classes => "classes",
            Self::ClassPolicy => "class policy",
            Self::RetryAfter => "retry_after_policy",
            Self::PermitCost => "permit_cost",
            Self::Fairness => "fairness",
            Self::Wfq => "WeightedFairQueue",
            Self::Drr => "DeficitRoundRobin",
            Self::DeadlineAware => "DeadlineAware",
            Self::Overcommit => "memory_overcommit_policy",
            Self::Degrade => "DegradeToLight",
            Self::Checkpoint => "checkpoint_policy",
            Self::Substrates => "extra_substrates",
            Self::Substrate => "substrate",
            Self::CapabilityLimits => "capability_limits",
            Self::Scalar => "scalar",
        }
    }

    fn field(self, key: &str) -> Option<Self> {
        match self {
            Self::Task => match key {
                "class" | "operation" | "root_operation_id" | "source" | "reason" => {
                    Some(Self::Scalar)
                }
                "scope" => Some(Self::Scope),
                "blocking_dispatch" => Some(Self::BlockingDispatch),
                "stages" => Some(Self::Stages),
                _ => None,
            },
            Self::Scope => (key == "Child").then_some(Self::Child),
            Self::Child => match key {
                "parent_operation_id" | "parent_stage" | "parent_awaits" => Some(Self::Scalar),
                _ => None,
            },
            Self::Stage => match key {
                "class" | "stage" | "substrate_hint" | "fan_out" => Some(Self::Scalar),
                "reduce_policy" => Some(Self::Reduce),
                _ => None,
            },
            Self::Reduce => match key {
                "stable_sort_key"
                | "duplicate_merge"
                | "tie_break"
                | "error_aggregation"
                | "partial_result_ordering" => Some(Self::Scalar),
                _ => None,
            },
            Self::BlockingDispatch => (key == "requested_stack").then_some(Self::RequestedStack),
            Self::RequestedStack => (key == "stack_size_bytes").then_some(Self::Scalar),
            Self::Config => match key {
                "topology" => Some(Self::Topology),
                "resources" => Some(Self::Resources),
                "classes" => Some(Self::Classes),
                "extra_substrates" => Some(Self::Substrates),
                "capability_limits" => Some(Self::CapabilityLimits),
                _ => None,
            },
            Self::Topology => match key {
                "cpu" => Some(Self::Cpu),
                "physical_domains" => Some(Self::PhysicalDomains),
                "blocking_threads"
                | "large_stack_slots"
                | "maintenance_workers"
                | "local_runtime_slots" => Some(Self::Scalar),
                _ => None,
            },
            Self::Cpu => match key {
                "mode" => Some(Self::CpuMode),
                "reserve_cores" | "min_workers" | "max_workers" => Some(Self::Scalar),
                _ => None,
            },
            Self::PhysicalDomains => match key {
                "shared_blocking" | "dedicated" => Some(Self::PhysicalMode),
                _ => None,
            },
            Self::CpuMode | Self::PhysicalMode => (key == "Fixed").then_some(Self::Scalar),
            Self::Resources => match key {
                "max_cpu_units"
                | "max_memory_units"
                | "per_request_max_cpu_units"
                | "per_request_max_memory_units" => Some(Self::Scalar),
                "memory_unit_scale" => Some(Self::MemoryScale),
                _ => None,
            },
            Self::MemoryScale => (key == "bytes_per_unit").then_some(Self::Scalar),
            Self::Classes => Some(Self::ClassPolicy),
            Self::ClassPolicy => match key {
                "max_inflight"
                | "max_queue_depth"
                | "memory_permit_mode"
                | "memory_release_policy"
                | "overflow_policy"
                | "cancellation_policy"
                | "best_effort" => Some(Self::Scalar),
                "permit_cost" => Some(Self::PermitCost),
                "fairness" => Some(Self::Fairness),
                "memory_overcommit_policy" => Some(Self::Overcommit),
                "retry_after_policy" => Some(Self::RetryAfter),
                "checkpoint_policy" => Some(Self::Checkpoint),
                _ => None,
            },
            Self::RetryAfter => (key == "FixedMs").then_some(Self::Scalar),
            Self::PermitCost => match key {
                "cpu_units" | "memory_units" => Some(Self::Scalar),
                _ => None,
            },
            Self::Fairness => match key {
                "WeightedFairQueue" => Some(Self::Wfq),
                "DeficitRoundRobin" => Some(Self::Drr),
                "DeadlineAware" => Some(Self::DeadlineAware),
                _ => None,
            },
            Self::Wfq => match key {
                "weight" | "burst" => Some(Self::Scalar),
                _ => None,
            },
            Self::Drr => (key == "quantum").then_some(Self::Scalar),
            Self::DeadlineAware => (key == "slack_ms").then_some(Self::Scalar),
            Self::Overcommit => (key == "DegradeToLight").then_some(Self::Degrade),
            Self::Degrade => (key == "fallback_class").then_some(Self::Scalar),
            Self::Checkpoint => match key {
                "every_n_work_items"
                | "before_fan_out"
                | "before_large_allocation"
                | "before_stage_boundary"
                | "before_reduce" => Some(Self::Scalar),
                _ => None,
            },
            Self::Substrate => match key {
                "name" | "kind" | "capability_pool" => Some(Self::Scalar),
                _ => None,
            },
            Self::CapabilityLimits => Some(Self::Scalar),
            Self::Stages | Self::Substrates | Self::Scalar => None,
        }
    }
}

struct ScanState {
    limits: StrictIngressLimits,
    failure: RefCell<Option<StrictIngressError>>,
}

impl ScanState {
    fn reject<E: serde::de::Error>(&self, error: StrictIngressError) -> E {
        *self.failure.borrow_mut() = Some(error);
        E::custom("strict ingress rejected")
    }
}

struct ScanSeed<'a> {
    state: &'a ScanState,
    shape: Shape,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for ScanSeed<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(ScanVisitor(self))
    }
}

struct ScanVisitor<'a>(ScanSeed<'a>);

impl<'de> Visitor<'de> for ScanVisitor<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a bounded {} JSON value", self.0.shape.name())
    }

    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_string<E: serde::de::Error>(self, _value: String) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let seed = self.0;
        if seed.depth > seed.state.limits.max_depth {
            return Err(seed.state.reject(StrictIngressError::DepthLimit {
                max: seed.state.limits.max_depth,
            }));
        }
        if matches!(
            seed.shape,
            Shape::Scalar | Shape::Stages | Shape::Substrates
        ) {
            return Err(seed
                .state
                .reject(StrictIngressError::InvalidShape(seed.shape.name())));
        }
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(seed.state.reject(StrictIngressError::DuplicateKey { key }));
            }
            let Some(shape) = seed.shape.field(&key) else {
                return Err(seed.state.reject(StrictIngressError::UnknownKey {
                    object: seed.shape.name(),
                    key,
                }));
            };
            map.next_value_seed(ScanSeed {
                state: seed.state,
                shape,
                depth: seed.depth + 1,
            })?;
        }
        Ok(())
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let seed = self.0;
        if seed.depth > seed.state.limits.max_depth {
            return Err(seed.state.reject(StrictIngressError::DepthLimit {
                max: seed.state.limits.max_depth,
            }));
        }
        let element = match seed.shape {
            Shape::Stages => Shape::Stage,
            Shape::Substrates => Shape::Substrate,
            _ => {
                return Err(seed
                    .state
                    .reject(StrictIngressError::InvalidShape(seed.shape.name())));
            }
        };
        let mut count = 0usize;
        loop {
            if seed.shape == Shape::Stages && count == seed.state.limits.max_stages {
                // The seed rejects as soon as the next array element begins;
                // no over-limit stage body is allocated or decoded.
                if seq
                    .next_element_seed(RejectStageSeed { state: seed.state })?
                    .is_none()
                {
                    break;
                }
            }
            if seq
                .next_element_seed(ScanSeed {
                    state: seed.state,
                    shape: element,
                    depth: seed.depth + 1,
                })?
                .is_none()
            {
                break;
            }
            count += 1;
        }
        Ok(())
    }
}

struct RejectStageSeed<'a> {
    state: &'a ScanState,
}

impl<'de> DeserializeSeed<'de> for RejectStageSeed<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, _deserializer: D) -> Result<(), D::Error> {
        Err(self.state.reject(StrictIngressError::StageLimit {
            max: self.state.limits.max_stages,
        }))
    }
}

fn scan(bytes: &[u8], limits: StrictIngressLimits, shape: Shape) -> Result<(), StrictIngressError> {
    for (name, value) in [
        ("max_bytes", limits.max_bytes),
        ("max_depth", limits.max_depth),
        ("max_stages", limits.max_stages),
    ] {
        if value == 0 {
            return Err(StrictIngressError::InvalidLimit(name));
        }
    }
    if bytes.len() > limits.max_bytes {
        return Err(StrictIngressError::ByteLimit {
            actual: bytes.len(),
            max: limits.max_bytes,
        });
    }
    let state = ScanState {
        limits,
        failure: RefCell::new(None),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let result = ScanSeed {
        state: &state,
        shape,
        depth: 1,
    }
    .deserialize(&mut deserializer)
    .and_then(|()| deserializer.end());
    if let Some(error) = state.failure.into_inner() {
        return Err(error);
    }
    result.map_err(|error| StrictIngressError::Decode(error.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictTaskInput {
    class: TaskClass,
    operation: String,
    root_operation_id: String,
    source: PlanSource,
    reason: ClassificationRationale,
    scope: StrictScope,
    #[serde(default)]
    blocking_dispatch: Option<StrictBlockingDispatch>,
    stages: Vec<StageDescriptor>,
}

#[derive(Deserialize)]
enum StrictScope {
    Root,
    Child(StrictChild),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_field_names,
    reason = "strict wire fields must match the published parent_* identifiers"
)]
struct StrictChild {
    parent_operation_id: String,
    parent_stage: TaskStage,
    parent_awaits: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StrictBlockingDispatch {
    SharedBlocking,
    RequestedStack(StrictRequestedStack),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictRequestedStack {
    stack_size_bytes: u64,
}

fn validate_requested_stack_size(size: u64, usize_max: u64) -> Result<u64, StrictIngressError> {
    if size == 0 || size > MAX_REQUESTED_STACK_BYTES || size > usize_max {
        return Err(StrictIngressError::BlockingDispatch(
            "requested stack size is not representable and within the supported bound",
        ));
    }
    Ok(size)
}

/// Decode bounded, strict task bytes and validate the resulting task plan.
pub fn parse_task_spec(
    bytes: &[u8],
    limits: StrictIngressLimits,
) -> Result<ValidatedTaskPlan, StrictIngressError> {
    scan(bytes, limits, Shape::Task)?;
    let input: StrictTaskInput = serde_json::from_slice(bytes)
        .map_err(|error| StrictIngressError::Decode(error.to_string()))?;
    let first = input.stages.first().map(|stage| stage.substrate_hint);
    let blocking_family = matches!(
        first,
        Some(
            SubstrateHint::BlockingPool
                | SubstrateHint::LargeStackCapability
                | SubstrateHint::BackgroundOnly
        )
    );
    let stack_size_bytes = match (blocking_family, input.blocking_dispatch) {
        (true, Some(StrictBlockingDispatch::SharedBlocking)) | (false, None) => None,
        (true, Some(StrictBlockingDispatch::RequestedStack(request))) => {
            Some(validate_requested_stack_size(
                request.stack_size_bytes,
                u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            )?)
        }
        (true, None) => {
            return Err(StrictIngressError::BlockingDispatch(
                "blocking-family task requires an explicit dispatch tag",
            ));
        }
        (false, Some(_)) => {
            return Err(StrictIngressError::BlockingDispatch(
                "non-blocking task must not declare blocking dispatch",
            ));
        }
    };
    let scope = match input.scope {
        StrictScope::Root => TaskScope::Root,
        StrictScope::Child(child) => TaskScope::Child {
            parent_operation_id: child.parent_operation_id,
            parent_stage: child.parent_stage,
            parent_awaits: child.parent_awaits,
        },
    };
    ValidatedTaskPlan::try_from(TaskSpec {
        class: input.class,
        operation: input.operation,
        root_operation_id: input.root_operation_id,
        source: input.source,
        reason: input.reason,
        scope,
        stack_size_bytes,
        stages: input.stages,
    })
    .map_err(StrictIngressError::TaskPlan)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictRuntimeInput {
    topology: TopologyConfig,
    resources: ResourceBudget,
    classes: BTreeMap<TaskClass, ClassPolicy>,
    #[serde(default)]
    extra_substrates: Vec<SubstrateRecord>,
    #[serde(default)]
    capability_limits: BTreeMap<String, u32>,
}

/// Parsed declaration that can only be constructed through the bounded bytes
/// entrypoint. It intentionally does not implement `Deserialize` directly.
pub struct StrictRuntimeConfig(StrictRuntimeInput);

impl StrictRuntimeConfig {
    /// Reuse the existing Builder's topology, policy, and inventory validation.
    /// Canonical built-in substrates are never replayed as extra registrations.
    pub fn into_builder(self) -> Builder {
        let input = self.0;
        let mut builder = Builder::new()
            .topology(input.topology)
            .resources(input.resources);
        for (class, policy) in input.classes {
            builder = builder.class_policy(class, policy);
        }
        for substrate in input.extra_substrates {
            builder = builder.substrate(substrate);
        }
        for (pool, slots) in input.capability_limits {
            builder = builder.capability_limit(pool, slots);
        }
        builder
    }
}

/// Decode bounded, strict deployment configuration bytes before Builder use.
pub fn parse_runtime_config(
    bytes: &[u8],
    limits: StrictIngressLimits,
) -> Result<StrictRuntimeConfig, StrictIngressError> {
    scan(bytes, limits, Shape::Config)?;
    serde_json::from_slice(bytes)
        .map(StrictRuntimeConfig)
        .map_err(|error| StrictIngressError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::{Error as _, Unexpected};

    #[test]
    fn requested_stack_size_validation_covers_32_bit_width() {
        let invalid = StrictIngressError::BlockingDispatch(
            "requested stack size is not representable and within the supported bound",
        );
        assert_eq!(
            validate_requested_stack_size(0, u64::MAX),
            Err(invalid.clone())
        );
        assert_eq!(
            validate_requested_stack_size(MAX_REQUESTED_STACK_BYTES, u64::MAX),
            Ok(MAX_REQUESTED_STACK_BYTES)
        );
        assert_eq!(
            validate_requested_stack_size(MAX_REQUESTED_STACK_BYTES + 1, u64::MAX),
            Err(invalid.clone())
        );
        assert_eq!(
            validate_requested_stack_size(u64::from(u32::MAX), u64::from(u32::MAX)),
            Ok(u64::from(u32::MAX))
        );
        assert_eq!(
            validate_requested_stack_size(u64::from(u32::MAX) + 1, u64::from(u32::MAX)),
            Err(invalid)
        );
    }

    #[test]
    fn scan_visitor_reports_the_expected_shape_in_serde_errors() {
        let state = ScanState {
            limits: StrictIngressLimits::default(),
            failure: RefCell::new(None),
        };

        for (shape, expected) in [
            (Shape::Task, "a bounded task JSON value"),
            (Shape::CpuMode, "a bounded cpu mode JSON value"),
            (Shape::ClassPolicy, "a bounded class policy JSON value"),
            (
                Shape::CapabilityLimits,
                "a bounded capability_limits JSON value",
            ),
        ] {
            let visitor = ScanVisitor(ScanSeed {
                state: &state,
                shape,
                depth: 0,
            });
            let error = serde::de::value::Error::invalid_type(Unexpected::Seq, &visitor);
            assert_eq!(
                error.to_string(),
                format!("invalid type: sequence, expected {expected}")
            );
        }
    }
}
