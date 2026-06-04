//! Aggregate runtime configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::policy::ClassPolicy;
use crate::resource::ResourceBudget;
use crate::task::TaskClass;
use crate::topology::TopologyConfig;

/// The full, validated configuration a runtime is built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    pub topology: TopologyConfig,
    pub resources: ResourceBudget,
    pub classes: BTreeMap<TaskClass, ClassPolicy>,
}
