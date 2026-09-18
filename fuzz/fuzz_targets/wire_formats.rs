//! Untrusted bytes through the published wire formats.
//!
//! `Snapshot` (schema 2), `TopologyConfig` and `ClassPolicy` are what a
//! dashboard, a collector or a config loader hands to `serde_json`. Arbitrary
//! input must be a typed parse error or a value — never a panic in a
//! `Deserialize` impl or in what a consumer does next with the value:
//!
//! * a `Snapshot` that parses answers `conservation_violation()` and survives
//!   a serialize → deserialize round trip unchanged (the decimal-string `u128`
//!   encoding is lossless in both directions);
//! * a `TopologyConfig` that parses answers `validate()`, `declared_slots()`
//!   and `try_resolved_cpu_workers()` for any core count;
//! * a `ClassPolicy` that parses round-trips and answers `is_disabled()`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use taskmesh_contract::{ClassPolicy, Snapshot, TopologyConfig};

fuzz_target!(|data: &[u8]| {
    if let Ok(snapshot) = serde_json::from_slice::<Snapshot>(data) {
        let _ = snapshot.conservation_violation();
        let json = serde_json::to_string(&snapshot).expect("a parsed snapshot serializes");
        let again: Snapshot = serde_json::from_str(&json).expect("serialized snapshot parses");
        assert_eq!(again, snapshot, "Snapshot round trip must be lossless");
    }
    if let Ok(topology) = serde_json::from_slice::<TopologyConfig>(data) {
        let verdict = topology.validate();
        let _ = topology.declared_slots();
        for available in [0usize, 1, 2, 7, 64, usize::MAX] {
            let resolved = topology.try_resolved_cpu_workers(available);
            if verdict.is_ok() {
                // A validated topology resolves a worker count for any host.
                assert!(
                    resolved.is_ok(),
                    "validated topology fails to resolve for {available} cores: {resolved:?}"
                );
            }
        }
        let json = serde_json::to_string(&topology).expect("a parsed topology serializes");
        let again: TopologyConfig =
            serde_json::from_str(&json).expect("serialized topology parses");
        assert_eq!(
            again, topology,
            "TopologyConfig round trip must be lossless"
        );
    }
    if let Ok(policy) = serde_json::from_slice::<ClassPolicy>(data) {
        let _ = policy.is_disabled();
        let json = serde_json::to_string(&policy).expect("a parsed policy serializes");
        let again: ClassPolicy = serde_json::from_str(&json).expect("serialized policy parses");
        assert_eq!(again, policy, "ClassPolicy round trip must be lossless");
    }
});
