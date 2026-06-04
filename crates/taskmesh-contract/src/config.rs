//! Aggregate runtime configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::policy::ClassPolicy;
use crate::resource::ResourceBudget;
use crate::task::TaskClass;
use crate::topology::{SubstrateRecord, TopologyConfig};

/// The full, validated configuration a runtime is built from.
///
/// This is the authoritative, serializable description of a built runtime: it
/// includes the resolved substrate inventory (built-ins plus any deployment
/// additions), so inspecting or persisting a `RuntimeConfig` faithfully
/// represents the runtime that was actually constructed — not a reduced view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    pub topology: TopologyConfig,
    pub resources: ResourceBudget,
    pub classes: BTreeMap<TaskClass, ClassPolicy>,
    /// The resolved substrate inventory (canonical built-ins + deployment
    /// additions), in registry order. Mirrors the governor snapshot's inventory.
    pub substrates: Vec<SubstrateRecord>,
}
