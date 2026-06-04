//! Resource budgets and the unit cost of a permit.

use serde::{Deserialize, Serialize};

/// The resource a single permit holds while inflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermitCost {
    pub cpu_units: u32,
    pub memory_units: u32,
}

/// Conversion primitive between measured bytes and abstract memory units.
/// Required (`> 0`) for `Measured`/`Hybrid` memory modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUnitScale {
    pub bytes_per_unit: u64,
}

impl Default for MemoryUnitScale {
    fn default() -> Self {
        Self { bytes_per_unit: 1 }
    }
}

impl MemoryUnitScale {
    /// Convert measured bytes to units, rounding up. Returns 0 when unscaled.
    pub fn units_for(&self, bytes: u64) -> u32 {
        if self.bytes_per_unit == 0 {
            return 0;
        }
        let units = bytes.div_ceil(self.bytes_per_unit);
        units.min(u32::MAX as u64) as u32
    }
}

/// Global resource ceilings for the runtime. A field of `0` means "no limit".
/// `memory_unit_scale` defaults to 1 byte/unit via [`MemoryUnitScale::default`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBudget {
    pub max_cpu_units: u32,
    pub max_memory_units: u32,
    pub per_request_max_cpu_units: u32,
    pub per_request_max_memory_units: u32,
    pub memory_unit_scale: MemoryUnitScale,
}

impl ResourceBudget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cpu_units(mut self, value: u32) -> Self {
        self.max_cpu_units = value;
        self
    }

    pub fn memory_units(mut self, value: u32) -> Self {
        self.max_memory_units = value;
        self
    }

    pub fn per_request_cpu_units(mut self, value: u32) -> Self {
        self.per_request_max_cpu_units = value;
        self
    }

    pub fn per_request_memory_units(mut self, value: u32) -> Self {
        self.per_request_max_memory_units = value;
        self
    }

    pub fn memory_unit_scale(mut self, bytes_per_unit: u64) -> Self {
        self.memory_unit_scale = MemoryUnitScale { bytes_per_unit };
        self
    }
}
