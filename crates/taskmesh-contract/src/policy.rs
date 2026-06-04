//! Semantic class policy vocabulary — how a class competes for execution.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::resource::PermitCost;
use crate::task::TaskClass;

/// Cross-class scheduling discipline a class participates in when contended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FairnessPolicy {
    Fifo,
    /// Weighted-fair share by virtual finish time. `weight` is honored; `burst`
    /// is **reserved** — a future burst-credit allowance — and is not yet read by
    /// the scheduler.
    WeightedFairQueue {
        weight: u32,
        burst: u32,
    },
    DeficitRoundRobin {
        quantum: u32,
    },
    DeadlineAware {
        slack_ms: u64,
    },
    BestEffortScavenger,
}

/// How memory permits are sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryPermitMode {
    /// Reserve the configured `permit_cost.memory_units`.
    Estimated,
    /// Convert measured bytes into units via `MemoryUnitScale`.
    Measured,
    /// Reserve the estimate, then reconcile up to measured usage.
    Hybrid,
}

/// When held memory units are returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryReleasePolicy {
    OnTaskCompletion,
    OnStageBoundary,
    LeakDetecting,
}

/// What to do when a class's memory reservation would overcommit the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryOvercommitPolicy {
    Reject,
    Queue,
    DegradeToLight { fallback_class: TaskClass },
}

/// Enforceable checkpoint metadata — inspected at well-defined hook points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    pub every_n_work_items: Option<u32>,
    pub before_fan_out: bool,
    pub before_large_allocation: bool,
    pub before_stage_boundary: bool,
    pub before_reduce: bool,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            every_n_work_items: None,
            before_fan_out: true,
            before_large_allocation: true,
            before_stage_boundary: true,
            before_reduce: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DuplicateMergePolicy {
    KeepFirstStable,
    KeepLastStable,
    StableFold,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieBreakPolicy {
    StableInputOrder,
    Lexicographic,
    DeterministicHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorAggregationPolicy {
    FirstStable,
    AllStable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartialResultOrdering {
    StableSortKey,
    StableStageOrder,
}

/// Reduce contract for parallel/fan-out stages. Every field is mandatory so a
/// parallel stage is never shippable with a nondeterministic reduce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicReducePolicy {
    pub stable_sort_key: Cow<'static, str>,
    pub duplicate_merge: DuplicateMergePolicy,
    pub tie_break: TieBreakPolicy,
    pub error_aggregation: ErrorAggregationPolicy,
    pub partial_result_ordering: PartialResultOrdering,
}

impl DeterministicReducePolicy {
    /// A complete, deterministic default keyed on a caller-provided stable key.
    pub fn keyed(stable_sort_key: impl Into<Cow<'static, str>>) -> Self {
        Self {
            stable_sort_key: stable_sort_key.into(),
            duplicate_merge: DuplicateMergePolicy::KeepFirstStable,
            tie_break: TieBreakPolicy::StableInputOrder,
            error_aggregation: ErrorAggregationPolicy::FirstStable,
            partial_result_ordering: PartialResultOrdering::StableSortKey,
        }
    }
}

/// What to do when admission cannot be granted immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverflowPolicy {
    Reject,
    QueueWithinDepth,
    /// Shed the request. Currently behaves as `Reject` (the engine rejects rather
    /// than silently dropping); a distinct shed/evict path is future work.
    DropBestEffort,
}

impl OverflowPolicy {
    pub fn is_queueable(self) -> bool {
        matches!(self, Self::QueueWithinDepth)
    }
}

/// Backpressure hint policy returned alongside a rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryAfterPolicy {
    None,
    FixedMs(u64),
    Adaptive,
}

/// Cancellation contract a class supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationPolicy {
    PreSubmitOnly,
    Cooperative,
    CooperativeWithDeadline,
}

/// The full governance policy for a single task class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassPolicy {
    pub max_inflight: u32,
    pub max_queue_depth: u32,
    pub permit_cost: PermitCost,
    pub fairness: FairnessPolicy,
    pub memory_permit_mode: MemoryPermitMode,
    pub memory_release_policy: MemoryReleasePolicy,
    pub memory_overcommit_policy: MemoryOvercommitPolicy,
    pub overflow_policy: OverflowPolicy,
    pub retry_after_policy: RetryAfterPolicy,
    pub cancellation_policy: CancellationPolicy,
    pub checkpoint_policy: CheckpointPolicy,
    pub best_effort: bool,
}

impl Default for ClassPolicy {
    fn default() -> Self {
        Self {
            max_inflight: u32::MAX,
            max_queue_depth: 0,
            permit_cost: PermitCost {
                cpu_units: 0,
                memory_units: 0,
            },
            fairness: FairnessPolicy::Fifo,
            memory_permit_mode: MemoryPermitMode::Estimated,
            memory_release_policy: MemoryReleasePolicy::OnTaskCompletion,
            memory_overcommit_policy: MemoryOvercommitPolicy::Reject,
            overflow_policy: OverflowPolicy::Reject,
            retry_after_policy: RetryAfterPolicy::None,
            cancellation_policy: CancellationPolicy::PreSubmitOnly,
            checkpoint_policy: CheckpointPolicy::default(),
            best_effort: false,
        }
    }
}

impl ClassPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_inflight(mut self, value: u32) -> Self {
        self.max_inflight = value;
        self
    }

    pub fn max_queue_depth(mut self, value: u32) -> Self {
        self.max_queue_depth = value;
        self
    }

    pub fn cpu_units(mut self, value: u32) -> Self {
        self.permit_cost.cpu_units = value;
        self
    }

    pub fn memory_units(mut self, value: u32) -> Self {
        self.permit_cost.memory_units = value;
        self
    }

    pub fn fairness(mut self, value: FairnessPolicy) -> Self {
        self.fairness = value;
        self
    }

    pub fn overflow_policy(mut self, value: OverflowPolicy) -> Self {
        self.overflow_policy = value;
        self
    }

    pub fn retry_after_policy(mut self, value: RetryAfterPolicy) -> Self {
        self.retry_after_policy = value;
        self
    }

    pub fn memory_permit_mode(mut self, value: MemoryPermitMode) -> Self {
        self.memory_permit_mode = value;
        self
    }

    pub fn memory_release_policy(mut self, value: MemoryReleasePolicy) -> Self {
        self.memory_release_policy = value;
        self
    }

    pub fn memory_overcommit_policy(mut self, value: MemoryOvercommitPolicy) -> Self {
        self.memory_overcommit_policy = value;
        self
    }

    pub fn cancellation_policy(mut self, value: CancellationPolicy) -> Self {
        self.cancellation_policy = value;
        self
    }

    pub fn checkpoint_policy(mut self, value: CheckpointPolicy) -> Self {
        self.checkpoint_policy = value;
        self
    }

    pub fn best_effort(mut self, value: bool) -> Self {
        self.best_effort = value;
        self
    }

    /// Whether this class is administratively disabled (admits nothing).
    pub fn is_disabled(&self) -> bool {
        self.max_inflight == 0
    }
}
