#[cfg(test)]
mod tests {
    mod disabled_soak {
        include!(concat!(env!("OUT_DIR"), "/disabled_soak.rs"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mixed_soak_assertions_pass_with_every_submission_rejected() {
        disabled_soak::unmodified_soak_control().await;
        disabled_soak::mixed_substrate_soak_with_concurrent_sweeper().await;
    }

    use std::{
        collections::BTreeMap,
        sync::Arc,
        time::{Duration, Instant},
    };
    use taskmesh::{ext::*, *};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn acquire_timeout_does_not_cover_synchronous_admission_lock_wait() {
        use std::sync::Barrier;
        struct SlowDrop {
            entered: Arc<Barrier>,
        }
        impl PermitWaker for SlowDrop {
            fn wake(&self) {}
        }
        impl Drop for SlowDrop {
            fn drop(&mut self) {
                self.entered.wait();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let a = TaskClass::new("a");
        let b = TaskClass::new("b");
        let rt = Arc::new(
            Builder::new()
                .class_policy(
                    a.clone(),
                    ClassPolicy::new()
                        .max_inflight(1)
                        .max_queue_depth(1)
                        .overflow_policy(OverflowPolicy::QueueWithinDepth),
                )
                .class_policy(b.clone(), ClassPolicy::new())
                .build()
                .unwrap(),
        );
        let holder = match rt
            .governor()
            .admit(&TaskSpec::io(a.clone()).operation("holder"))
        {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("{other:?}"),
        };
        let entered = Arc::new(Barrier::new(2));
        let ticket = match rt.governor().admit_waitable(
            &TaskSpec::io(a).operation("queued"),
            Arc::new(SlowDrop {
                entered: entered.clone(),
            }),
        ) {
            AdmissionDecision::Queued { ticket } => ticket,
            other => panic!("{other:?}"),
        };
        let abandon = {
            let rt = rt.clone();
            std::thread::spawn(move || rt.governor().abandon(ticket))
        };
        entered.wait();
        let start = Instant::now();
        let result = rt
            .run_io_with(
                TaskSpec::io(b).operation("late-start"),
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(5)),
                async { Ok::<_, ()>(start.elapsed()) },
            )
            .await
            .unwrap();
        assert!(
            result >= Duration::from_millis(80),
            "job started after acquisition budget: {result:?}"
        );
        abandon.join().unwrap();
        rt.governor().release(holder);
    }

    #[test]
    fn queued_waker_drop_reenters_governor_under_lock() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            sync::Weak,
        };
        const CHILD: &str = "TASKMESH_SEP16_WAKER_DROP_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct ReentrantDrop(Weak<Governor>);
            impl PermitWaker for ReentrantDrop {
                fn wake(&self) {}
            }
            impl Drop for ReentrantDrop {
                fn drop(&mut self) {
                    println!("ENTERED_REENTRANT_WAKER_DROP");
                    std::io::stdout().flush().unwrap();
                    let _ = self.0.upgrade().unwrap().snapshot();
                }
            }
            let c = TaskClass::new("c");
            let g = Arc::new(
                Governor::new(
                    PolicySet::new(
                        ResourceBudget::new(),
                        BTreeMap::from([(
                            c.clone(),
                            ClassPolicy::new()
                                .max_inflight(1)
                                .max_queue_depth(1)
                                .overflow_policy(OverflowPolicy::QueueWithinDepth),
                        )]),
                    ),
                    Arc::new(ManualClock::new(0)),
                )
                .unwrap(),
            );
            assert!(matches!(
                g.admit(&TaskSpec::io(c.clone()).operation("holder")),
                AdmissionDecision::Admitted { .. }
            ));
            let ticket = match g.admit_waitable(
                &TaskSpec::io(c).operation("queued"),
                Arc::new(ReentrantDrop(Arc::downgrade(&g))),
            ) {
                AdmissionDecision::Queued { ticket } => ticket,
                other => panic!("{other:?}"),
            };
            g.abandon(ticket);
            panic!("abandon unexpectedly completed instead of deadlocking");
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::queued_waker_drop_reenters_governor_under_lock",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child did not remain blocked: {status}");
            }
            if start.elapsed() >= Duration::from_secs(2) {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(String::from_utf8(output.stdout)
                    .unwrap()
                    .contains("ENTERED_REENTRANT_WAKER_DROP"));
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn requested_stack_class_with_nul_panics_before_spawn_error_mapping() {
        let c = TaskClass::new("class\0name");
        let rt = Builder::new()
            .class_policy(c.clone(), ClassPolicy::new())
            .build()
            .unwrap();
        let result = tokio::spawn(async move {
            rt.run_blocking(
                TaskSpec::blocking(c)
                    .operation("nul-thread-name")
                    .stack_size_bytes(2 * 1024 * 1024),
                || Ok::<_, ()>(()),
            )
            .await
        })
        .await;
        assert!(
            result.unwrap_err().is_panic(),
            "expected a host panic, not a typed RunError"
        );
    }

    #[test]
    fn admission_timestamp_precedes_actual_lease_commit() {
        use std::sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Barrier,
        };
        struct PausedClock {
            first: AtomicBool,
            now: AtomicU64,
            captured: Barrier,
            resume: Barrier,
        }
        impl Clock for PausedClock {
            fn now_ms(&self) -> u64 {
                let captured = self.now.load(Ordering::SeqCst);
                if self.first.swap(false, Ordering::SeqCst) {
                    // Model preemption after reading wall time, before locking state.
                    self.captured.wait();
                    self.resume.wait();
                }
                captured
            }
        }
        let clock = Arc::new(PausedClock {
            first: AtomicBool::new(true),
            now: AtomicU64::new(0),
            captured: Barrier::new(2),
            resume: Barrier::new(2),
        });
        let c = TaskClass::new("c");
        let g = Arc::new(
            Governor::new(
                PolicySet::new(
                    ResourceBudget::new(),
                    BTreeMap::from([(
                        c.clone(),
                        ClassPolicy::new()
                            .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
                    )]),
                ),
                clock.clone(),
            )
            .unwrap(),
        );
        let delayed = {
            let g = g.clone();
            let c = c.clone();
            std::thread::spawn(move || g.admit(&TaskSpec::io(c).operation("delayed")))
        };
        clock.captured.wait();
        clock.now.store(1_000, Ordering::SeqCst);
        let current = match g.admit(&TaskSpec::io(c.clone()).operation("current")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("{other:?}"),
        };
        clock.resume.wait();
        let just_committed = match delayed.join().unwrap() {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("{other:?}"),
        };
        assert_eq!(g.snapshot().classes[&c].inflight, 2);
        assert_eq!(g.reap_leaks_with(100).reclaimed_permits, 1);
        assert!(!g.reconcile_memory(just_committed, 0));
        assert!(g.reconcile_memory(current, 0));
        // A delayed reconcile can overwrite a newer successful heartbeat too.
        clock.first.store(true, Ordering::SeqCst);
        let old_heartbeat = {
            let g = g.clone();
            std::thread::spawn(move || g.reconcile_memory(current, 0))
        };
        clock.captured.wait();
        clock.now.store(2_000, Ordering::SeqCst);
        assert!(g.reconcile_memory(current, 0));
        clock.resume.wait();
        assert!(old_heartbeat.join().unwrap());
        assert_eq!(g.reap_leaks_with(100).reclaimed_permits, 1);
        assert_eq!(g.snapshot().classes[&c].inflight, 0);
    }

    struct InlineCpu;
    impl CpuExecutor for InlineCpu {
        fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
            work();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inline_cpu_work_runs_before_relative_timer_starts() {
        let c = TaskClass::new("c");
        let rt = Builder::new()
            .cpu_executor(Arc::new(InlineCpu))
            .class_policy(
                c.clone(),
                ClassPolicy::new().cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
            )
            .build()
            .unwrap();
        let start = Instant::now();
        let out = rt
            .run_cpu_with(
                TaskSpec::cpu(c).operation("inline"),
                SubmitOptions::unbounded().with_deadline(Duration::from_millis(5)),
                || {
                    std::thread::sleep(Duration::from_millis(60));
                    Ok::<_, ()>(7)
                },
            )
            .await;
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert!(matches!(out, Ok(7)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_stack_request_does_not_take_large_stack_slot() {
        let c = TaskClass::new("c");
        let rt = Builder::new()
            .topology(TopologyConfig::new().large_stack_slots(1))
            .class_policy(c.clone(), ClassPolicy::new().max_inflight(4))
            .build()
            .unwrap();
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for i in 0..2 {
            let rt = rt.clone();
            let c = c.clone();
            let active = active.clone();
            let max = max.clone();
            jobs.push(tokio::spawn(async move {
                rt.run_blocking(
                    TaskSpec::blocking(c)
                        .operation(format!("stack-{i}"))
                        .stack_size_bytes(2 * 1024 * 1024),
                    move || {
                        let n = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        max.fetch_max(n, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(80));
                        active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        Ok::<_, ()>(())
                    },
                )
                .await
            }));
        }
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        assert_eq!(max.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn requested_stack_async_deadline_waits_for_blocking_child_shutdown() {
        let c = TaskClass::new("c");
        let rt = Builder::new()
            .class_policy(
                c.clone(),
                ClassPolicy::new().cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
            )
            .build()
            .unwrap();
        let start = Instant::now();
        let out = rt
            .run_async_with_requested_stack_with(
                TaskSpec::base(c, SubstrateHint::LargeStackCapability)
                    .operation("child-shutdown")
                    .stack_size_bytes(2 * 1024 * 1024),
                SubmitOptions::unbounded()
                    .with_absolute_deadline(start + Duration::from_millis(20)),
                || async {
                    tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(150)));
                    std::future::pending::<Result<(), ()>>().await
                },
            )
            .await;
        assert!(matches!(
            out,
            Err(RunError::Governor(GovernorError::DeadlineExceeded))
        ));
        assert!(
            start.elapsed() >= Duration::from_millis(100),
            "yielding root still waits child shutdown past deadline"
        );
    }

    fn queue_policy(weight: u32) -> ClassPolicy {
        ClassPolicy::new()
            .max_inflight(64)
            .max_queue_depth(64)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .fairness(FairnessPolicy::WeightedFairQueue { weight, burst: 0 })
    }

    #[test]
    fn drr_empty_queue_retains_credit_across_busy_periods() {
        fn order_after(warmups: usize) -> Vec<char> {
            let a = TaskClass::new("a");
            let b = TaskClass::new("b");
            let policy = |quantum| {
                ClassPolicy::new()
                    .max_inflight(64)
                    .max_queue_depth(64)
                    .cpu_units(1)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth)
                    .fairness(FairnessPolicy::DeficitRoundRobin { quantum })
            };
            let g = Governor::new(
                PolicySet::new(
                    ResourceBudget::new().cpu_units(1),
                    BTreeMap::from([(a.clone(), policy(2)), (b.clone(), policy(1))]),
                ),
                Arc::new(ManualClock::new(0)),
            )
            .unwrap();
            let mut holder = match g.admit(&TaskSpec::io(b.clone()).operation("holder")) {
                AdmissionDecision::Admitted { permit_id } => permit_id,
                other => panic!("{other:?}"),
            };
            let queue = |class: &TaskClass, name: String| match g
                .admit(&TaskSpec::io(class.clone()).operation(name))
            {
                AdmissionDecision::Queued { ticket } => ticket,
                other => panic!("{other:?}"),
            };
            for i in 0..warmups {
                let ta = queue(&a, format!("warm-a-{i}"));
                let tb = queue(&b, format!("warm-b-{i}"));
                g.release(holder);
                let pa = g.claim(ta).expect("A gets the next ring visit");
                assert_eq!(g.snapshot().classes[&a].queued, 0);
                g.release(pa);
                holder = g.claim(tb).expect("B runs after A drains");
            }
            let mut pending = Vec::new();
            for i in 0..10 {
                pending.push(('a', queue(&a, format!("burst-a-{i}"))));
                pending.push(('b', queue(&b, format!("burst-b-{i}"))));
            }
            let mut order = Vec::new();
            while !pending.is_empty() {
                g.release(holder);
                let (index, permit) = pending
                    .iter()
                    .enumerate()
                    .find_map(|(i, (_, ticket))| g.claim(*ticket).map(|p| (i, p)))
                    .expect("exactly one queued request should be promoted");
                order.push(pending.remove(index).0);
                holder = permit;
            }
            g.release(holder);
            assert!(g
                .snapshot()
                .classes
                .values()
                .all(|c| c.inflight == 0 && c.queued == 0));
            order
        }
        let fresh = order_after(0);
        let warmed = order_after(20);
        println!("DRR fresh={fresh:?}; after 20 drained busy periods={warmed:?}");
        assert_eq!(&fresh[..3], &['a', 'a', 'b']);
        assert_eq!(
            &warmed[..10],
            &['a'; 10],
            "observed residual credit lets A drain its burst before B gets any service"
        );
    }

    #[test]
    fn large_wfq_weights_collapse_ratio_into_fifo() {
        fn first_a(a_weight: u32, b_weight: u32) -> bool {
            let a = TaskClass::new("a");
            let b = TaskClass::new("b");
            let g = Governor::new(
                PolicySet::new(
                    ResourceBudget::new().cpu_units(1),
                    BTreeMap::from([
                        (a.clone(), queue_policy(a_weight)),
                        (b.clone(), queue_policy(b_weight)),
                    ]),
                ),
                Arc::new(ManualClock::new(0)),
            )
            .unwrap();
            let holder = match g.admit(&TaskSpec::io(b.clone()).operation("holder")) {
                AdmissionDecision::Admitted { permit_id } => permit_id,
                x => panic!("{x:?}"),
            };
            let a_ticket = match g.admit(&TaskSpec::io(a).operation("a1")) {
                AdmissionDecision::Queued { ticket } => ticket,
                x => panic!("{x:?}"),
            };
            assert!(matches!(
                g.admit(&TaskSpec::io(b).operation("b1")),
                AdmissionDecision::Queued { .. }
            ));
            g.release(holder);
            g.claim(a_ticket).is_some()
        }
        assert!(!first_a(1, 2));
        assert!(
            first_a(2_000_000, 4_000_000),
            "same weight ratio now selects earlier a by FIFO"
        );
    }

    #[test]
    fn abandoned_wfq_work_leaves_phantom_debt() {
        let a = TaskClass::new("a");
        let b = TaskClass::new("b");
        let g = Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(1),
                BTreeMap::from([(a.clone(), queue_policy(1)), (b.clone(), queue_policy(1))]),
            ),
            Arc::new(ManualClock::new(0)),
        )
        .unwrap();
        let holder = match g.admit(&TaskSpec::io(b.clone()).operation("holder")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            x => panic!("{x:?}"),
        };
        for i in 0..10 {
            let ticket = match g.admit(&TaskSpec::io(a.clone()).operation(format!("canceled-{i}")))
            {
                AdmissionDecision::Queued { ticket } => ticket,
                x => panic!("{x:?}"),
            };
            g.abandon(ticket);
        }
        let a_ticket = match g.admit(&TaskSpec::io(a).operation("a1")) {
            AdmissionDecision::Queued { ticket } => ticket,
            x => panic!("{x:?}"),
        };
        let b_ticket = match g.admit(&TaskSpec::io(b).operation("b1")) {
            AdmissionDecision::Queued { ticket } => ticket,
            x => panic!("{x:?}"),
        };
        g.release(holder);
        assert!(
            g.claim(b_ticket).is_some(),
            "later equal-weight b beats a with canceled debt"
        );
        assert!(g.claim(a_ticket).is_none());
    }

    #[test]
    fn estimated_reconcile_resurrects_released_reservation() {
        let c = TaskClass::new("c");
        let classes = BTreeMap::from([(
            c.clone(),
            ClassPolicy::new()
                .memory_units(8)
                .memory_permit_mode(MemoryPermitMode::Estimated)
                .memory_release_policy(MemoryReleasePolicy::OnStageBoundary),
        )]);
        let g = Governor::new(
            PolicySet::new(ResourceBudget::new().memory_units(100), classes),
            Arc::new(ManualClock::new(0)),
        )
        .unwrap();
        let p = match g.admit(&TaskSpec::io(c.clone()).operation("root")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            x => panic!("{x:?}"),
        };
        assert_eq!(g.release_stage_memory(p, 6), 6);
        assert_eq!(g.snapshot().classes[&c].memory_units_held, 2);
        assert!(g.reconcile_memory(p, 2));
        assert_eq!(g.snapshot().classes[&c].memory_units_held, 8);
    }

    #[test]
    fn leak_sweep_keeps_a_claimable_dead_ticket() {
        let c = TaskClass::new("c");
        let clock = Arc::new(ManualClock::new(0));
        let classes = BTreeMap::from([(
            c.clone(),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
        )]);
        let g = Governor::new(
            PolicySet::new(ResourceBudget::new(), classes),
            clock.clone(),
        )
        .unwrap();
        let p = match g.admit(&TaskSpec::io(c.clone()).operation("holder")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            x => panic!("{x:?}"),
        };
        let ticket = match g.admit(&TaskSpec::io(c.clone()).operation("queued")) {
            AdmissionDecision::Queued { ticket } => ticket,
            x => panic!("{x:?}"),
        };
        g.release(p);
        clock.advance(11);
        let sweep =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.reap_leaks_with(10)));
        if cfg!(debug_assertions) {
            assert!(sweep.is_err(), "debug invariant must catch dead mapping");
        } else {
            assert_eq!(sweep.unwrap().reclaimed_permits, 1);
        }
        let dead = g
            .claim(ticket)
            .expect("observed bug: reclaimed ticket remains claimable");
        assert!(g.permit_provenance(dead).is_none());
        assert_eq!(g.snapshot().classes[&c].inflight, 0);
    }

    #[test]
    fn saturation_at_u32_max_admits_over_budget() {
        let c = TaskClass::new("c");
        let cost = 1u32 << 31;
        let classes = BTreeMap::from([(
            c.clone(),
            ClassPolicy::new().max_inflight(3).cpu_units(cost),
        )]);
        let g = Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(u32::MAX), classes),
            Arc::new(ManualClock::new(0)),
        )
        .unwrap();
        for name in ["a", "b"] {
            assert!(matches!(
                g.admit(&TaskSpec::io(c.clone()).operation(name)),
                AdmissionDecision::Admitted { .. }
            ));
        }
        assert_eq!(g.snapshot().classes[&c].inflight, 2);
        assert!(u64::from(cost) * 2 > u64::from(u32::MAX));
    }

    #[test]
    fn stage_release_does_not_refresh_leak_activity() {
        let c = TaskClass::new("c");
        let classes = BTreeMap::from([(
            c.clone(),
            ClassPolicy::new()
                .memory_units(10)
                .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
        )]);
        let clock = Arc::new(ManualClock::new(0));
        let g = Governor::new(
            PolicySet::new(ResourceBudget::new().memory_units(100), classes),
            clock.clone(),
        )
        .unwrap();
        let p = match g.admit(&TaskSpec::io(c).operation("root")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("{other:?}"),
        };
        clock.advance(59_999);
        assert_eq!(g.release_stage_memory(p, 1), 1);
        clock.advance(2);
        assert_eq!(
            g.reap_leaks_with(60_000).reclaimed_permits,
            1,
            "recent stage activity was not recorded and permit was reclaimed"
        );
    }

    #[test]
    fn invalid_topology_panics_in_fallible_build() {
        let result = std::panic::catch_unwind(|| {
            Builder::new()
                .topology(TopologyConfig::new().min_workers(8).max_workers(2))
                .build()
        });
        assert!(result.is_err(), "observed bug must be a panic, not Err");
    }

    #[test]
    fn default_policy_boots_without_builtin_inventory() {
        let mut policy = PolicySet::default();
        policy
            .classes
            .insert(TaskClass::new("c"), ClassPolicy::new());
        let g = Governor::new(policy, Arc::new(ManualClock::new(0))).unwrap();
        assert!(g.snapshot().substrates.is_empty());
        assert!(matches!(
            g.admit(&TaskSpec::io(TaskClass::new("c"))),
            AdmissionDecision::Admitted { .. }
        ));
    }

    #[test]
    fn completion_only_memory_policy_allows_early_stage_release() {
        let c = TaskClass::new("c");
        let classes = BTreeMap::from([(
            c.clone(),
            ClassPolicy::new()
                .memory_units(10)
                .memory_release_policy(MemoryReleasePolicy::OnTaskCompletion),
        )]);
        let g = Governor::new(
            PolicySet::new(ResourceBudget::new().memory_units(100), classes),
            Arc::new(ManualClock::new(0)),
        )
        .unwrap();
        let p = match g.admit(&TaskSpec::io(c.clone()).operation("root")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("{other:?}"),
        };
        assert_eq!(g.release_stage_memory(p, 10), 10);
        assert_eq!(g.snapshot().classes[&c].inflight, 1);
        assert_eq!(g.snapshot().classes[&c].memory_units_held, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_runfor_ignores_deadline() {
        let c = TaskClass::new("c");
        let rt = Builder::new()
            .class_policy(
                c.clone(),
                ClassPolicy::new().cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
            )
            .build()
            .unwrap();
        let started = Instant::now();
        let out = rt
            .run_blocking_with(
                TaskSpec::blocking(c).operation("slow"),
                SubmitOptions::unbounded().with_deadline(Duration::from_millis(5)),
                || {
                    std::thread::sleep(Duration::from_millis(60));
                    Ok::<_, ()>(7)
                },
            )
            .await;
        assert!(started.elapsed() >= Duration::from_millis(50));
        assert!(
            matches!(out, Ok(7)),
            "observed bug: success after deadline: {out:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn substrate_waiters_bypass_reject_policy_and_queue_bound() {
        let c = TaskClass::new("c");
        let rt = Builder::new()
            .topology(TopologyConfig::new().blocking_threads(1))
            .class_policy(
                c.clone(),
                ClassPolicy::new()
                    .max_inflight(1)
                    .max_queue_depth(0)
                    .overflow_policy(OverflowPolicy::Reject),
            )
            .build()
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = tokio::spawn({
            let rt = rt.clone();
            let c = c.clone();
            async move {
                rt.run_blocking(TaskSpec::blocking(c).operation("holder"), move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<_, ()>(())
                })
                .await
            }
        });
        started_rx.await.unwrap();
        let mut waiters = Vec::new();
        for i in 0..32 {
            let rt = rt.clone();
            let c = c.clone();
            waiters.push(tokio::spawn(async move {
                rt.run_blocking(
                    TaskSpec::blocking(c).operation(format!("waiter-{i}")),
                    || Ok::<_, ()>(()),
                )
                .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            waiters.iter().all(|w| !w.is_finished()),
            "Reject policy did not reject at saturation"
        );
        assert_eq!(rt.snapshot().classes[&c].queued, 0);
        release_tx.send(()).unwrap();
        holder.await.unwrap().unwrap();
        for w in waiters {
            w.await.unwrap().unwrap();
        }
    }
}
