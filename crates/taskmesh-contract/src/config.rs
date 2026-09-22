//! Aggregate runtime configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::policy::ClassPolicy;
use crate::resource::ResourceBudget;
use crate::task::TaskClass;
use crate::topology::{SubstrateRecord, TopologyConfig};

/// The validated declaration a runtime was built from, plus its resolved
/// substrate inventory.
///
/// Topology modes remain as declared (for example, `CpuMode::Auto` is not
/// rewritten to a machine-specific fixed value). Enforced capability limits
/// and the installed executor descriptor are runtime state exposed by the
/// governor snapshot and the host runtime, respectively; they are deliberately
/// not duplicated in this portable declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    pub topology: TopologyConfig,
    pub resources: ResourceBudget,
    pub classes: BTreeMap<TaskClass, ClassPolicy>,
    /// The resolved substrate inventory (canonical built-ins + deployment
    /// additions), in registry order. Mirrors the governor snapshot's inventory.
    pub substrates: Vec<SubstrateRecord>,
}
