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
        use Shape as S;
        match self {
            S::Task => match key {
                "class" | "operation" | "root_operation_id" | "source" | "reason" => {
                    Some(S::Scalar)
                }
                "scope" => Some(S::Scope),
                "blocking_dispatch" => Some(S::BlockingDispatch),
                "stages" => Some(S::Stages),
                _ => None,
            },
            S::Scope => (key == "Child").then_some(S::Child),
            S::Child => match key {
                "parent_operation_id" | "parent_stage" | "parent_awaits" => Some(S::Scalar),
                _ => None,
            },
            S::Stage => match key {
                "class" | "stage" | "substrate_hint" | "fan_out" => Some(S::Scalar),
                "reduce_policy" => Some(S::Reduce),
                _ => None,
            },
            S::Reduce => match key {
                "stable_sort_key"
                | "duplicate_merge"
                | "tie_break"
                | "error_aggregation"
                | "partial_result_ordering" => Some(S::Scalar),
                _ => None,
            },
            S::BlockingDispatch => (key == "requested_stack").then_some(S::RequestedStack),
            S::RequestedStack => (key == "stack_size_bytes").then_some(S::Scalar),
            S::Config => match key {
                "topology" => Some(S::Topology),
                "resources" => Some(S::Resources),
                "classes" => Some(S::Classes),
                "extra_substrates" => Some(S::Substrates),
                "capability_limits" => Some(S::CapabilityLimits),
                _ => None,
            },
            S::Topology => match key {
                "cpu" => Some(S::Cpu),
                "physical_domains" => Some(S::PhysicalDomains),
                "blocking_threads"
                | "large_stack_slots"
                | "maintenance_workers"
                | "local_runtime_slots" => Some(S::Scalar),
                _ => None,
            },
            S::Cpu => match key {
                "mode" => Some(S::CpuMode),
                "reserve_cores" | "min_workers" | "max_workers" => Some(S::Scalar),
                _ => None,
            },
            S::CpuMode => (key == "Fixed").then_some(S::Scalar),
            S::PhysicalDomains => match key {
                "shared_blocking" | "dedicated" => Some(S::PhysicalMode),
                _ => None,
            },
            S::PhysicalMode => (key == "Fixed").then_some(S::Scalar),
            S::Resources => match key {
                "max_cpu_units"
                | "max_memory_units"
                | "per_request_max_cpu_units"
                | "per_request_max_memory_units" => Some(S::Scalar),
                "memory_unit_scale" => Some(S::MemoryScale),
                _ => None,
            },
            S::MemoryScale => (key == "bytes_per_unit").then_some(S::Scalar),
            S::Classes => Some(S::ClassPolicy),
            S::ClassPolicy => match key {
                "max_inflight"
                | "max_queue_depth"
                | "memory_permit_mode"
                | "memory_release_policy"
                | "overflow_policy"
                | "cancellation_policy"
                | "best_effort" => Some(S::Scalar),
                "permit_cost" => Some(S::PermitCost),
                "fairness" => Some(S::Fairness),
                "memory_overcommit_policy" => Some(S::Overcommit),
                "retry_after_policy" => Some(S::RetryAfter),
                "checkpoint_policy" => Some(S::Checkpoint),
                _ => None,
            },
            S::RetryAfter => (key == "FixedMs").then_some(S::Scalar),
            S::PermitCost => match key {
                "cpu_units" | "memory_units" => Some(S::Scalar),
                _ => None,
            },
            S::Fairness => match key {
                "WeightedFairQueue" => Some(S::Wfq),
                "DeficitRoundRobin" => Some(S::Drr),
                "DeadlineAware" => Some(S::DeadlineAware),
                _ => None,
            },
            S::Wfq => match key {
                "weight" | "burst" => Some(S::Scalar),
                _ => None,
            },
            S::Drr => (key == "quantum").then_some(S::Scalar),
            S::DeadlineAware => (key == "slack_ms").then_some(S::Scalar),
            S::Overcommit => (key == "DegradeToLight").then_some(S::Degrade),
            S::Degrade => (key == "fallback_class").then_some(S::Scalar),
            S::Checkpoint => match key {
                "every_n_work_items"
                | "before_fan_out"
                | "before_large_allocation"
                | "before_stage_boundary"
                | "before_reduce" => Some(S::Scalar),
                _ => None,
            },
            S::Substrate => match key {
                "name" | "kind" | "capability_pool" => Some(S::Scalar),
                _ => None,
            },
            S::CapabilityLimits => Some(S::Scalar),
            S::Stages | S::Substrates | S::Scalar => None,
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
        (true, Some(StrictBlockingDispatch::SharedBlocking)) => None,
        (true, Some(StrictBlockingDispatch::RequestedStack(request))) => {
            let size = request.stack_size_bytes;
            if size == 0
                || size > MAX_REQUESTED_STACK_BYTES
                || size > u64::try_from(usize::MAX).unwrap_or(u64::MAX)
            {
                return Err(StrictIngressError::BlockingDispatch(
                    "requested stack size is not representable and within the supported bound",
                ));
            }
            Some(size)
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
        (false, None) => None,
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
pub struct StrictRuntimeConfig {
    topology: TopologyConfig,
    resources: ResourceBudget,
    classes: BTreeMap<TaskClass, ClassPolicy>,
    #[serde(default)]
    extra_substrates: Vec<SubstrateRecord>,
    #[serde(default)]
    capability_limits: BTreeMap<String, u32>,
}

impl StrictRuntimeConfig {
    /// Reuse the existing Builder's topology, policy, and inventory validation.
    /// Canonical built-in substrates are never replayed as extra registrations.
    pub fn into_builder(self) -> Builder {
        let mut builder = Builder::new()
            .topology(self.topology)
            .resources(self.resources);
        for (class, policy) in self.classes {
            builder = builder.class_policy(class, policy);
        }
        for substrate in self.extra_substrates {
            builder = builder.substrate(substrate);
        }
        for (pool, slots) in self.capability_limits {
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
    serde_json::from_slice(bytes).map_err(|error| StrictIngressError::Decode(error.to_string()))
}
