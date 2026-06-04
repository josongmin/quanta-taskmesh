//! Shared host-runtime test fixtures.
#![allow(dead_code)]

use taskmesh::*;

/// A runtime with one queueable class `c`.
pub fn runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

pub fn blocking() -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("c")).operation("op")
}

pub fn io() -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation("op")
}

pub fn cpu() -> TaskSpec {
    TaskSpec::cpu(TaskClass::new("c")).operation("op")
}

pub fn local() -> TaskSpec {
    TaskSpec::local(TaskClass::new("c")).operation("op")
}
