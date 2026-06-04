//! Observable runtime state.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::task::TaskClass;
use crate::topology::SubstrateRecord;

/// Per-class accounting at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassSnapshot {
    pub inflight: u32,
    pub queued: u32,
    pub cpu_units_held: u32,
    pub memory_units_held: u32,
}

/// A consistent view of governed state: per-class accounting plus the substrate
/// inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub classes: BTreeMap<TaskClass, ClassSnapshot>,
    pub substrates: Vec<SubstrateRecord>,
}
