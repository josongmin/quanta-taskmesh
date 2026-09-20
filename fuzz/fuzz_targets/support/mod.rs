use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

static SEEN: OnceLock<Mutex<BTreeSet<(&'static str, &'static str)>>> = OnceLock::new();

/// Emit each semantic witness once per libFuzzer process. The producer runner
/// enables this explicitly and refuses a receipt when a required witness is
/// absent; ordinary fuzzing does not pay for log traffic on every input.
pub(crate) fn checkpoint(target: &str, checkpoint: &str) {
    if std::env::var_os("TASKMESH_FUZZ_WITNESS").as_deref() != Some("1".as_ref()) {
        return;
    }
    let mut seen = SEEN
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .expect("fuzz checkpoint mutex is not poisoned");
    let target: &'static str = match target {
        "admission_churn" => "admission_churn",
        "policy_topology" => "policy_topology",
        "wire_formats" => "wire_formats",
        _ => panic!("unregistered fuzz target {target}"),
    };
    let checkpoint: &'static str = match checkpoint {
        "target_entry" => "target_entry",
        "governor_valid" => "governor_valid",
        "transition_step" => "transition_step",
        "quiescent" => "quiescent",
        "policy_valid" => "policy_valid",
        "topology_valid" => "topology_valid",
        "snapshot_valid" => "snapshot_valid",
        "class_policy_valid" => "class_policy_valid",
        _ => panic!("unregistered fuzz checkpoint {checkpoint}"),
    };
    if seen.insert((target, checkpoint)) {
        eprintln!("taskmesh-fuzz-checkpoint target={target} checkpoint={checkpoint}");
    }
}
