//! Task identity and specification — the product-neutral request vocabulary.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::policy::DeterministicReducePolicy;
use crate::topology::SubstrateHint;

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

/// Origin of an execution plan. Product-neutral provenance for audit/telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanSource {
    PublicSdk,
    FluentSdk,
    SearchAdapter,
    Warmup,
    Indexing,
    Internal,
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
    Child { parent_stage: TaskStage },
}

/// One stage in a task's execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDescriptor {
    pub class: TaskClass,
    pub stage: TaskStage,
    pub substrate_hint: SubstrateHint,
    /// Whether this stage fans out into parallel work. A fan-out stage is not
    /// shippable without a `reduce_policy` (enforced by the composite feature).
    pub fan_out: bool,
    /// Required for parallel/fan-out stages; validated by the composite feature.
    pub reduce_policy: Option<DeterministicReducePolicy>,
}

/// A fully-described request submitted to the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub class: TaskClass,
    pub operation: String,
    pub root_operation_id: String,
    pub source: PlanSource,
    pub reason: ClassificationRationale,
    pub scope: TaskScope,
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
            source: PlanSource::Internal,
            reason: ClassificationRationale::ExplicitMapping,
            scope: TaskScope::Root,
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
    pub fn child_of(
        mut self,
        root_operation_id: impl Into<String>,
        parent_stage: TaskStage,
    ) -> Self {
        self.root_operation_id = root_operation_id.into();
        self.scope = TaskScope::Child { parent_stage };
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
}
