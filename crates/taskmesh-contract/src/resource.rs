//! Resource budgets and the unit cost of a permit.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Why a measured byte reading cannot be expressed in memory units.
///
/// Conversion is a *checked* boundary: an unrepresentable reading is a typed
/// accounting fault, never a saturated number that quietly understates real
/// usage (which is what lets an over-budget request look admissible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceConversionError {
    /// `bytes_per_unit == 0`: the scale cannot convert anything.
    UnscaledMemoryUnits,
    /// The converted unit count does not fit the `u32` unit domain.
    MemoryUnitsOverflow { bytes: u64, bytes_per_unit: u64 },
}

impl fmt::Display for ResourceConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnscaledMemoryUnits => {
                f.write_str("memory unit scale has bytes_per_unit == 0; cannot convert bytes")
            }
            Self::MemoryUnitsOverflow {
                bytes,
                bytes_per_unit,
            } => write!(
                f,
                "measured {bytes} bytes at {bytes_per_unit} bytes/unit exceeds the u32 unit domain"
            ),
        }
    }
}

impl std::error::Error for ResourceConversionError {}

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
    /// Convert measured bytes to units, rounding up.
    ///
    /// Checked on both edges: an unscaled (`bytes_per_unit == 0`) scale and a
    /// reading larger than the `u32` unit domain are typed errors. Saturating
    /// here would report *less* memory than a task actually holds, which is
    /// exactly how an over-budget admission hides itself.
    pub fn units_for(&self, bytes: u64) -> Result<u32, ResourceConversionError> {
        if self.bytes_per_unit == 0 {
            return Err(ResourceConversionError::UnscaledMemoryUnits);
        }
        let units = bytes.div_ceil(self.bytes_per_unit);
        // The `TryFromIntError` carries nothing the caller cannot already see, so
        // the typed error names the inputs instead of wrapping it.
        u32::try_from(units).map_err(|_unrepresentable| {
            ResourceConversionError::MemoryUnitsOverflow {
                bytes,
                bytes_per_unit: self.bytes_per_unit,
            }
        })
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
