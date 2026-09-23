//! Audit M2: GovernorError/RunError expose Display + std::error::Error — and
//! every `Display` on the error surface renders the text a caller's log will
//! actually carry. Each variant is pinned to its exact rendering: a variant
//! whose message silently changed, or two variants that render alike, is a
//! diagnostic that no longer says what happened.

use std::error::Error;

use taskmesh_contract::*;

#[test]
fn run_error_display_and_source() {
    #[derive(Debug)]
    struct MyErr;
    impl std::fmt::Display for MyErr {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl Error for MyErr {}

    let task: RunError<MyErr> = RunError::Task(MyErr);
    assert!(task.to_string().contains("boom"));
    assert!(task.source().is_some());

    let gov: RunError<MyErr> = RunError::Governor(GovernorError::LocalRuntimeUnavailable);
    assert!(gov.to_string().contains("local runtime"));
}

#[test]
fn run_error_boolean_classification_is_exclusive() {
    let governor: RunError<&str> = RunError::Governor(GovernorError::Cancelled);
    assert!(governor.is_governor());
    assert!(!governor.is_task());

    let task: RunError<&str> = RunError::Task("failure");
    assert!(!task.is_governor());
    assert!(task.is_task());
}

// ---- exact renderings -------------------------------------------------------

#[test]
fn topology_errors_render_their_shape() {
    assert_eq!(
        format!(
            "{}",
            TopologyError::InvertedCpuWorkerBounds {
                min_workers: 8,
                max_workers: 2,
            }
        ),
        "topology cpu.min_workers (8) exceeds cpu.max_workers (2)"
    );
    assert_eq!(
        format!(
            "{}",
            TopologyError::SlotCountTooLarge {
                pool: "blocking",
                slots: 5_000_000_000,
                max: MAX_CAPABILITY_SLOTS,
            }
        ),
        format!("topology pool blocking declares 5000000000 slots, above the governed maximum {MAX_CAPABILITY_SLOTS}")
    );
    assert_eq!(
        format!("{}", TopologyError::ZeroFixedCpuWorkers),
        "topology cpu.mode = Fixed(0) cannot execute any work"
    );
    assert_eq!(
        format!(
            "{}",
            TopologyError::ExecutorDeclaresFewerWorkers {
                declared: 2,
                resolved: 4,
            }
        ),
        "cpu executor declares 2 worker(s) but the topology resolved 4; the cpu gate cannot exceed the executor"
    );
}

#[test]
fn resource_conversion_errors_render_the_reading() {
    assert_eq!(
        format!("{}", ResourceConversionError::UnscaledMemoryUnits),
        "memory unit scale has bytes_per_unit == 0; cannot convert bytes"
    );
    assert_eq!(
        format!(
            "{}",
            ResourceConversionError::MemoryUnitsOverflow {
                bytes: u64::MAX,
                bytes_per_unit: 1,
            }
        ),
        format!(
            "measured {} bytes at 1 bytes/unit exceeds the u32 unit domain",
            u64::MAX
        )
    );
}

#[test]
fn governor_errors_render_one_line_each() {
    assert_eq!(
        format!(
            "{}",
            GovernorError::Rejected(AdmissionVerdict::QueueFull {
                retry_after_ms: Some(5)
            })
        ),
        "admission rejected: queue full"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::TicketClaimTerminated {
                ticket: 7,
                reason: TerminalReason::Reclaimed,
            }
        ),
        "queued ticket 7 terminated: reclaimed by the leak sweep"
    );
    assert_eq!(
        format!("{}", GovernorError::InvalidTicketClaim { ticket: 9 }),
        "queued ticket 9 is unknown to the governor"
    );
    assert_eq!(
        format!("{}", GovernorError::LocalRuntimeUnavailable),
        "local runtime substrate unavailable"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::PolicyViolation("class c degrades to itself".into())
        ),
        "policy violation: class c degrades to itself"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::InvalidTopology(TopologyError::ZeroFixedCpuWorkers)
        ),
        "invalid topology: topology cpu.mode = Fixed(0) cannot execute any work"
    );
    assert_eq!(
        format!("{}", GovernorError::Cancelled),
        "work cooperatively cancelled"
    );
    assert_eq!(
        format!("{}", GovernorError::DeadlineExceeded),
        "work exceeded its run deadline"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::DeadlineUnsupported {
                class: TaskClass::new("c"),
                policy: CancellationPolicy::PreSubmitOnly,
            }
        ),
        "class c (cancellation policy PreSubmitOnly) cannot enforce a run deadline"
    );
    assert_eq!(
        format!("{}", GovernorError::LeaseReclaimed { permit_id: 3 }),
        "permit 3 was reclaimed before the work was dispatched"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::WorkerUnavailable {
                context: "run_blocking".into(),
                detail: "EAGAIN".to_string(),
            }
        ),
        "run_blocking: worker could not be created: EAGAIN"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::WorkerPanicked {
                context: "cpu executor".into(),
            }
        ),
        "cpu executor panicked"
    );
    assert_eq!(
        format!(
            "{}",
            GovernorError::JobAbandoned {
                context: "cpu executor".into(),
            }
        ),
        "cpu executor went away without reporting a result"
    );
}

#[test]
fn admission_verdicts_and_terminal_reasons_render_one_line_each() {
    let verdicts = [
        (AdmissionVerdict::Admitted, "admitted"),
        (
            AdmissionVerdict::QueueFull {
                retry_after_ms: None,
            },
            "queue full",
        ),
        (
            AdmissionVerdict::CpuSaturated {
                retry_after_ms: None,
            },
            "cpu saturated",
        ),
        (
            AdmissionVerdict::MemorySaturated {
                retry_after_ms: None,
            },
            "memory saturated",
        ),
        (AdmissionVerdict::ClassDisabled, "class disabled"),
        (
            AdmissionVerdict::UnknownClass {
                class: TaskClass::new("ghost"),
            },
            "unknown class: ghost",
        ),
        (AdmissionVerdict::RuntimeUnavailable, "runtime unavailable"),
        (
            AdmissionVerdict::ClassificationFailed,
            "classification failed",
        ),
        (
            AdmissionVerdict::PermitAcquireTimedOut {
                retry_after_ms: None,
            },
            "permit acquire timed out",
        ),
        (
            AdmissionVerdict::CancelledBeforeSubmit,
            "cancelled before submit",
        ),
        (
            AdmissionVerdict::DeadlineExpiredBeforeSubmit,
            "deadline expired before submit",
        ),
        (
            AdmissionVerdict::RecursiveAdmission,
            "recursive admission rejected",
        ),
        (AdmissionVerdict::MalformedTask, "malformed task spec"),
        (
            AdmissionVerdict::SubstrateMismatch,
            "substrate hint / run-path mismatch",
        ),
        (
            AdmissionVerdict::SubstratePoolTimedOut {
                retry_after_ms: None,
            },
            "substrate pool acquire timed out",
        ),
        (
            AdmissionVerdict::SubstrateSaturated {
                retry_after_ms: None,
            },
            "substrate capability pool saturated",
        ),
        (
            AdmissionVerdict::NestedWaitCycle {
                held_by_root: HeldCapacity::CapabilityPool {
                    pool: "blocking".to_string(),
                },
            },
            "declared nested wait cycle: capability pool blocking is held entirely by the child's own root",
        ),
    ];
    for (verdict, rendered) in verdicts {
        assert_eq!(format!("{verdict}"), rendered, "{verdict:?}");
    }
    assert_eq!(
        format!("{}", TerminalReason::Released),
        "released before the claim"
    );
    assert_eq!(
        format!("{}", TerminalReason::Abandoned),
        "abandoned by the waiter"
    );
}

#[test]
fn run_errors_prefix_their_side() {
    let governor: RunError<String> = RunError::Governor(GovernorError::Cancelled);
    assert_eq!(
        format!("{governor}"),
        "governor error: work cooperatively cancelled"
    );
    let task: RunError<String> = RunError::Task("disk full".to_string());
    assert_eq!(format!("{task}"), "task error: disk full");
}

// ---- accessors ---------------------------------------------------------------

#[test]
fn only_the_admitted_verdict_is_admitted() {
    assert!(AdmissionVerdict::Admitted.is_admitted());
    for verdict in [
        AdmissionVerdict::QueueFull {
            retry_after_ms: Some(1),
        },
        AdmissionVerdict::ClassDisabled,
        AdmissionVerdict::RecursiveAdmission,
        AdmissionVerdict::SubstrateSaturated {
            retry_after_ms: None,
        },
    ] {
        assert!(!verdict.is_admitted(), "{verdict:?} is not an admission");
    }
}

#[test]
fn as_verdict_reaches_the_rejection_and_nothing_else() {
    let verdict = AdmissionVerdict::CpuSaturated {
        retry_after_ms: Some(25),
    };
    let rejected = GovernorError::Rejected(verdict.clone());
    assert_eq!(rejected.as_verdict(), Some(&verdict));
    assert_eq!(rejected.retry_after_ms(), Some(25));
    assert_eq!(GovernorError::Cancelled.as_verdict(), None);
    assert_eq!(GovernorError::DeadlineExceeded.retry_after_ms(), None);

    let run: RunError<String> = RunError::Governor(rejected);
    assert_eq!(run.as_verdict(), Some(&verdict));
    assert_eq!(run.retry_after_ms(), Some(25));
    assert!(run.is_governor());
    assert!(!run.is_task());
    let task: RunError<String> = RunError::Task("boom".to_string());
    assert_eq!(task.as_verdict(), None);
    assert_eq!(task.retry_after_ms(), None);
    assert_eq!(task.task(), Some(&"boom".to_string()));
    assert_eq!(task.governor(), None);
}

// ---- the `?` chain and the hint accessor -------------------------------------

#[test]
fn every_backpressure_verdict_carries_its_hint_and_no_other_verdict_does() {
    // `retry_after_ms()` is what a caller's backoff reads. Every verdict that
    // *has* a hint must surface it — a missing arm would tell a caller "no
    // hint" for a rejection the engine sized a retry for — and no verdict
    // without one may invent it.
    let hinted = [
        AdmissionVerdict::QueueFull {
            retry_after_ms: Some(7),
        },
        AdmissionVerdict::CpuSaturated {
            retry_after_ms: Some(7),
        },
        AdmissionVerdict::MemorySaturated {
            retry_after_ms: Some(7),
        },
        AdmissionVerdict::PermitAcquireTimedOut {
            retry_after_ms: Some(7),
        },
        AdmissionVerdict::SubstratePoolTimedOut {
            retry_after_ms: Some(7),
        },
        AdmissionVerdict::SubstrateSaturated {
            retry_after_ms: Some(7),
        },
    ];
    for verdict in &hinted {
        assert_eq!(
            verdict.retry_after_ms(),
            Some(7),
            "{verdict:?} carries a retry hint and must surface it"
        );
        assert_eq!(
            GovernorError::Rejected(verdict.clone()).retry_after_ms(),
            Some(7),
            "the hint must survive wrapping in GovernorError::Rejected"
        );
    }
    let unhinted = [
        AdmissionVerdict::UnknownClass {
            class: TaskClass::new("x"),
        },
        AdmissionVerdict::SubstrateMismatch,
        AdmissionVerdict::MalformedTask,
        AdmissionVerdict::CancelledBeforeSubmit,
        AdmissionVerdict::RuntimeUnavailable,
        AdmissionVerdict::NestedWaitCycle {
            held_by_root: HeldCapacity::CpuBudget,
        },
    ];
    for verdict in &unhinted {
        assert_eq!(
            verdict.retry_after_ms(),
            None,
            "{verdict:?} has no hint to give"
        );
    }
}

#[test]
fn errors_convert_along_the_question_mark_chain_and_keep_their_source() {
    // A consumer writes `topology.validate()?; runtime.run_io(..).await?` in a
    // function returning `Result<_, RunError<E>>`. That compiles only through
    // `From<TopologyError> for GovernorError` and `From<GovernorError> for
    // RunError<E>`, and the original error must still be reachable through
    // `source()` — otherwise `?` would flatten the diagnosis.
    #[derive(Debug)]
    struct MyErr;
    impl std::fmt::Display for MyErr {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("boom")
        }
    }
    impl Error for MyErr {}

    fn validate_then_run() -> Result<(), RunError<MyErr>> {
        let topology = TopologyConfig::new()
            .cpu_fixed(4)
            .min_workers(8)
            .max_workers(2);
        let checked: Result<(), GovernorError> = topology.validate().map_err(GovernorError::from);
        checked?;
        Ok(())
    }
    let error = validate_then_run().expect_err("an inverted worker window fails validation");
    let RunError::Governor(GovernorError::InvalidTopology(topology_error)) = &error else {
        panic!("`?` must keep the typed topology error, got {error:?}");
    };
    // source() chains: RunError -> GovernorError -> TopologyError.
    let governor: &GovernorError = error
        .source()
        .and_then(|s| s.downcast_ref::<GovernorError>())
        .expect("RunError::Governor exposes the governor error as its source");
    let topology_source = governor
        .source()
        .and_then(|s| s.downcast_ref::<TopologyError>())
        .expect("GovernorError::InvalidTopology exposes the topology error as its source");
    assert_eq!(topology_source, topology_error);
    // The task side of the accessor pair is empty for a governor error, and
    // a governor error without an inner error has no source.
    assert!(
        error.task().is_none(),
        "a governor error is not a task error"
    );
    assert!(!error.is_task(), "and is_task agrees");
    let plain: RunError<MyErr> = RunError::Governor(GovernorError::Cancelled);
    assert!(
        plain.source().is_some(),
        "Governor(_) always exposes the governor error"
    );
    assert!(
        GovernorError::Cancelled.source().is_none(),
        "a leaf governor error has no source"
    );
}

#[test]
fn execution_phases_render_as_their_snapshot_keys() {
    // The phase names are wire-facing (dashboards key on them); pin the exact
    // spelling of every variant so a rename cannot slip through as a
    // "cosmetic" change.
    let rendered: Vec<String> = [
        ExecutionPhase::DispatchReserved,
        ExecutionPhase::Accepted,
        ExecutionPhase::Running,
        ExecutionPhase::CleanupPending,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        rendered,
        [
            "dispatch_reserved",
            "accepted",
            "running",
            "cleanup_pending"
        ],
        "phase names are wire keys; every variant renders exactly"
    );
}
