//! Feature slices. Each is a vertical capability with its own pure `domain`
//! rules and stateful `service` over the shared `GovernedState`.

pub mod admission;
pub mod composite;
pub mod fairness;
pub mod inventory;
pub mod memory;
