//! Task identity and specification — the product-neutral request vocabulary.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::policy::DeterministicReducePolicy;
use crate::topology::SubstrateHint;
use crate::validation::{
    validate_identifier, validate_task_spec, TaskIdentifierField, TaskPlanError, ValidatedTaskPlan,
};

/// Stable semantic class of a unit of work. String-like identifier: ordered and
/// hashable so it can key deterministic `BTreeMap` state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskClass(Cow<'static, str>);

impl TaskClass {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for TaskClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A named stage within a (possibly multi-stage) task.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskStage(Cow<'static, str>);

impl TaskStage {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for TaskStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bounded, product-neutral provenance key for audit and telemetry.
///
/// The wire representation remains a JSON string. Product-specific mapping is
/// owned by adapters; core code treats this value as opaque provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanSource(Cow<'static, str>);

impl PlanSource {
    /// Canonical source for work originated inside taskmesh.
    pub const INTERNAL: Self = Self(Cow::Borrowed("internal"));

    /// Builds a validated opaque provenance key.
    pub fn new(value: impl Into<Cow<'static, str>>) -> Result<Self, TaskPlanError> {
        let value = value.into();
        validate_identifier(TaskIdentifierField::PlanSource, value.as_ref())?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }

    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; use PlanSource::INTERNAL before the next breaking release"
    )]
    pub const Internal: Self = Self(Cow::Borrowed("Internal"));
    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; map product provenance in the consumer adapter before the next breaking release"
    )]
    pub const PublicSdk: Self = Self(Cow::Borrowed("PublicSdk"));
    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; map product provenance in the consumer adapter before the next breaking release"
    )]
    pub const FluentSdk: Self = Self(Cow::Borrowed("FluentSdk"));
    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; map product provenance in the consumer adapter before the next breaking release"
    )]
    pub const SearchAdapter: Self = Self(Cow::Borrowed("SearchAdapter"));
    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; map product provenance in the consumer adapter before the next breaking release"
    )]
    pub const Warmup: Self = Self(Cow::Borrowed("Warmup"));
    #[allow(
        non_upper_case_globals,
        reason = "legacy 0.2 migration alias keeps its published spelling"
    )]
    #[deprecated(
        note = "accepted during the 0.2 migration window; map product provenance in the consumer adapter before the next breaking release"
    )]
    pub const Indexing: Self = Self(Cow::Borrowed("Indexing"));
}

impl fmt::Display for PlanSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for PlanSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PlanSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Why a task carries the class it does. Keeps classification auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassificationRationale {
    ExplicitMapping,
    DerivedFromRequestKind,
    DerivedFromStageMap,
}

/// Scope of a task in a composite execution tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskScope {
    Root,
    Child {
        /// Exact operation identity of the immediate parent.
        parent_operation_id: String,
        parent_stage: TaskStage,
        /// The parent's execution blocks on this child's result (declared with
        /// [`TaskSpec::awaited_child_of`]). While the parent holds its permit
        /// it cannot free capacity, so a child that could only wait for
        /// capacity held entirely by its own root is refused with
        /// [`crate::AdmissionVerdict::NestedWaitCycle`] instead of being queued
        /// into a deadlock (ADR 0003 D12). A wait that is not declared is
        /// never inferred: an undeclared child queues as any request does.
        #[serde(default)]
        parent_awaits: bool,
    },
}

/// One stage in a task's execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDescriptor {
    pub class: TaskClass,
    pub stage: TaskStage,
    pub substrate_hint: SubstrateHint,
    /// Whether this stage fans out into parallel work. A fan-out stage is not
    /// valid without a `reduce_policy` (enforced by the contract validator).
    pub fan_out: bool,
    /// Required for parallel/fan-out stages; validated by the contract validator.
    pub reduce_policy: Option<DeterministicReducePolicy>,
}

/// A fully-described request submitted to the runtime.
///
/// Scope note — `stages` is a **declared governance plan, not a host-walked
/// execution plan.** taskmesh is an admission/governance control-plane: each
/// `run_*` submission executes the *caller's* closure on the spec's primary
/// substrate ([`TaskSpec::primary_substrate_hint`], the first stage). The
/// runtime does not iterate later stages or schedule a closure per stage — the
/// caller drives execution. What the declared stages *are* authoritative for is
/// governance: shape validation (every stage's `class` must match the task
/// class), deterministic-reduce enforcement on `fan_out` stages, and recursion /
/// root-attribution lineage (via the child's declared `parent_stage`). Treat
/// stages beyond the first as a governance-validated declaration, not as steps
/// the runtime will run for you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub class: TaskClass,
    pub operation: String,
    pub root_operation_id: String,
    pub source: PlanSource,
    pub reason: ClassificationRationale,
    pub scope: TaskScope,
    #[serde(default)]
    pub stack_size_bytes: Option<u64>,
    pub stages: Vec<StageDescriptor>,
}

impl TaskSpec {
    pub fn io(class: TaskClass) -> Self {
        Self::base(class, SubstrateHint::AsyncIo)
    }

    pub fn blocking(class: TaskClass) -> Self {
        Self::base(class, SubstrateHint::BlockingPool)
    }

    pub fn cpu(class: TaskClass) -> Self {
        Self::base(class, SubstrateHint::SharedCpuExecutor)
    }

    pub fn local(class: TaskClass) -> Self {
        Self::base(class, SubstrateHint::LocalRuntime)
    }

    pub fn base(class: TaskClass, hint: SubstrateHint) -> Self {
        let bootstrap_stage = TaskStage::new(match hint {
            SubstrateHint::AsyncIo => "io",
            SubstrateHint::BlockingPool => "blocking",
            SubstrateHint::SharedCpuExecutor => "cpu",
            SubstrateHint::LargeStackCapability => "large_stack",
            SubstrateHint::LocalRuntime => "local_runtime",
            SubstrateHint::BackgroundOnly => "background",
        });
        Self {
            class: class.clone(),
            operation: String::new(),
            root_operation_id: String::new(),
            source: PlanSource::INTERNAL,
            reason: ClassificationRationale::ExplicitMapping,
            scope: TaskScope::Root,
            stack_size_bytes: None,
            stages: vec![StageDescriptor {
                class,
                stage: bootstrap_stage,
                substrate_hint: hint,
                fan_out: false,
                reduce_policy: None,
            }],
        }
    }

    /// Sets the human-facing operation name. For a **root** task this also
    /// becomes the admission/attribution key (`root_operation_id`). For a
    /// **child** (already reparented via [`TaskSpec::child_of`]) the inherited
    /// root id is preserved — naming the child's own operation must never
    /// re-root it, so `child_of(root).operation(name)` keeps `root`.
    pub fn operation(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if matches!(self.scope, TaskScope::Root) {
            self.root_operation_id = name.clone();
        }
        self.operation = name;
        self
    }

    /// Reparents this task under an existing root. The child inherits the root's
    /// `root_operation_id` so its permits roll up to the root's accounting bucket.
    ///
    /// The relationship is lineage only: the engine does not assume the parent
    /// waits for this child. If it does, say so with
    /// [`Self::awaited_child_of`], which is what turns an otherwise certain
    /// deadlock into a typed refusal.
    pub fn child_of(
        mut self,
        root_operation_id: impl Into<String>,
        parent_operation_id: impl Into<String>,
        parent_stage: TaskStage,
    ) -> Self {
        self.root_operation_id = root_operation_id.into();
        self.scope = TaskScope::Child {
            parent_operation_id: parent_operation_id.into(),
            parent_stage,
            parent_awaits: false,
        };
        self
    }

    /// Like [`Self::child_of`], and declares that the parent's execution blocks
    /// on this child's result.
    ///
    /// A parent that awaits its child keeps its own permit — and every unit of
    /// capacity that permit holds — until the child has run. If the capacity
    /// this child needs is held *entirely* by its own root, no release can ever
    /// come: the child would wait on its parent, which waits on the child. The
    /// engine refuses such a submission before admission with
    /// [`crate::AdmissionVerdict::NestedWaitCycle`] (naming the capacity)
    /// instead of queueing it. Capacity shared with other holders is not a
    /// cycle — they can finish — and is queued for as usual (ADR 0003 D12).
    pub fn awaited_child_of(
        mut self,
        root_operation_id: impl Into<String>,
        parent_operation_id: impl Into<String>,
        parent_stage: TaskStage,
    ) -> Self {
        self.root_operation_id = root_operation_id.into();
        self.scope = TaskScope::Child {
            parent_operation_id: parent_operation_id.into(),
            parent_stage,
            parent_awaits: true,
        };
        self
    }

    /// Appends a sequential stage carrying the task's class and substrate hint.
    pub fn stage(mut self, stage: TaskStage, hint: SubstrateHint) -> Self {
        self.stages.push(StageDescriptor {
            class: self.class.clone(),
            stage,
            substrate_hint: hint,
            fan_out: false,
            reduce_policy: None,
        });
        self
    }

    /// Declares the requested stack size for the bootstrap execution substrate.
    ///
    /// The current Tokio host only consumes this on blocking-family substrates.
    /// Other hosts may reject or ignore it according to their own policy.
    pub fn stack_size_bytes(mut self, stack_size_bytes: u64) -> Self {
        self.stack_size_bytes = Some(stack_size_bytes);
        self
    }

    /// Appends a parallel/fan-out stage that carries its deterministic reduce policy.
    pub fn reduce_stage(
        mut self,
        stage: TaskStage,
        hint: SubstrateHint,
        reduce_policy: DeterministicReducePolicy,
    ) -> Self {
        self.stages.push(StageDescriptor {
            class: self.class.clone(),
            stage,
            substrate_hint: hint,
            fan_out: true,
            reduce_policy: Some(reduce_policy),
        });
        self
    }

    /// Appends a fan-out stage *without* a reduce policy. Such a spec is invalid
    /// and is rejected by reduce validation — useful to express the negative case.
    pub fn fan_out_stage(mut self, stage: TaskStage, hint: SubstrateHint) -> Self {
        self.stages.push(StageDescriptor {
            class: self.class.clone(),
            stage,
            substrate_hint: hint,
            fan_out: true,
            reduce_policy: None,
        });
        self
    }

    /// The substrate hint of the first (bootstrap) stage.
    pub fn primary_substrate_hint(&self) -> SubstrateHint {
        self.stages
            .first()
            .map_or(SubstrateHint::AsyncIo, |s| s.substrate_hint)
    }

    pub fn requested_stack_size_bytes(&self) -> Option<u64> {
        self.stack_size_bytes
    }

    /// Validates this raw wire/builder value and returns an admission-ready plan.
    pub fn validate(&self) -> Result<ValidatedTaskPlan, TaskPlanError> {
        ValidatedTaskPlan::try_from(self.clone())
    }

    /// Validate a borrowed plan without copying its owned fields. This is the
    /// same contract check used by [`Self::validate`], for admission paths that
    /// consume the plan only for the duration of one transition.
    pub fn validate_borrowed(&self) -> Result<(), TaskPlanError> {
        validate_task_spec(self)
    }
}
