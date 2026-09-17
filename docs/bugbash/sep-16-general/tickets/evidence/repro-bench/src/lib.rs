#[cfg(test)]
mod tests {
    mod portable_iai {
        include!(concat!(env!("OUT_DIR"), "/iai_bodies.rs"));
    }
    use taskmesh_bench::{
        loadgen::simulate,
        workload::{
            fixture, generate_bursty, retrieval_policy, trace_from_csv, Arrival, BurstConfig,
        },
    };

    #[test]
    fn zero_dwell_burst_config_never_advances_phase_boundary() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        const CHILD: &str = "TASKMESH_AUDIT_ZERO_DWELL_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let cfg = BurstConfig {
                count: 1,
                mean_off_secs: 0.0,
                mean_on_secs: 0.0,
                ..BurstConfig::default()
            };
            println!("ENTERING_ZERO_DWELL_GENERATOR");
            std::io::stdout().flush().unwrap();
            generate_bursty(&cfg);
            panic!("zero dwell generator unexpectedly returned");
        }
        // Positive dwell control completes without requiring any timeout.
        assert_eq!(
            generate_bursty(&BurstConfig {
                count: 1,
                ..BurstConfig::default()
            })
            .len(),
            1
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::zero_dwell_burst_config_never_advances_phase_boundary",
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
                panic!("expected zero-dwell nonprogress, got {status}");
            }
            if start.elapsed() >= Duration::from_secs(1) {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(String::from_utf8(output.stdout)
                    .unwrap()
                    .contains("ENTERING_ZERO_DWELL_GENERATOR"));
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn iai_function_bodies_destroy_input_governor_before_return() {
        use std::{
            collections::BTreeMap,
            sync::{
                atomic::{AtomicBool, AtomicUsize, Ordering},
                Arc,
            },
        };
        use taskmesh::{
            ext::{Clock, Governor, PolicySet},
            ClassPolicy, ResourceBudget, TaskClass, TaskSpec,
        };
        struct DropClock {
            active: Arc<AtomicBool>,
            inside_drops: Arc<AtomicUsize>,
        }
        impl Clock for DropClock {
            fn now_ms(&self) -> u64 {
                0
            }
        }
        impl Drop for DropClock {
            fn drop(&mut self) {
                if self.active.load(Ordering::SeqCst) {
                    self.inside_drops.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        for path in ["roundtrip", "reject", "snapshot"] {
            let active = Arc::new(AtomicBool::new(false));
            let drops = Arc::new(AtomicUsize::new(0));
            let c = TaskClass::new("c");
            let g = Governor::new(
                PolicySet::new(
                    ResourceBudget::new(),
                    BTreeMap::from([(c.clone(), ClassPolicy::new())]),
                ),
                Arc::new(DropClock {
                    active: active.clone(),
                    inside_drops: drops.clone(),
                }),
            )
            .unwrap();
            active.store(true, Ordering::SeqCst);
            match path {
                "roundtrip" => {
                    let _returned =
                        portable_iai::admit_release((g, TaskSpec::io(c).operation("op")));
                    active.store(false, Ordering::SeqCst);
                }
                "reject" => {
                    let _returned = portable_iai::admit_unknown_reject((
                        g,
                        TaskSpec::io(TaskClass::new("ghost")).operation("op"),
                    ));
                    active.store(false, Ordering::SeqCst);
                }
                "snapshot" => {
                    let _returned = portable_iai::snapshot(g);
                    active.store(false, Ordering::SeqCst);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                drops.load(Ordering::SeqCst),
                1,
                "{path} destroys governor before caller resumes"
            );
        }
    }

    #[test]
    fn contention_batch_count_differs_from_criterion_iterations() {
        // Source-guarded integer diagnostic; not a Criterion timing qualification.
        let source =
            include_str!("../../../../../../../crates/taskmesh-bench/benches/contention.rs");
        assert!(source.contains("let ops_per_thread = (iters as usize / t).max(1);"));
        assert!(source.contains("Duration::from_secs_f64(total / tput)"));
        let actual = |iters: usize, t: usize| (iters / t).max(1) * t;
        assert_eq!(actual(1, 8), 8);
        assert_eq!(actual(9, 8), 8);
        assert_eq!(actual(8, 8), 8);
    }

    #[test]
    fn trace_parser_accepts_invalid_event_times() {
        for text in ["NaN,retrieval", "-1,retrieval", "2,retrieval\n1,retrieval"] {
            let arrivals = trace_from_csv(text).expect("invalid timestamp accepted");
            let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
            let (_, result) = simulate(&fx, &arrivals, 10, 1_000_000_000);
            assert!(
                result.is_conserved(),
                "invalid schedule still reports conserved"
            );
        }
        let arrivals = trace_from_csv("inf,retrieval").expect("infinite timestamp accepted");
        let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            simulate(&fx, &arrivals, 10, 1_000_000_000)
        }));
        if cfg!(debug_assertions) {
            assert!(
                outcome.is_err(),
                "infinite timestamp overflows completion time"
            );
        } else {
            assert!(
                outcome.unwrap().1.is_conserved(),
                "wrapped completion time still reports conserved"
            );
        }
    }

    #[test]
    fn open_loop_waits_are_recorded_more_than_once() {
        let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
        let arrivals = vec![
            Arrival {
                send_time_secs: 0.0,
                class: "retrieval".into(),
            },
            Arrival {
                send_time_secs: 0.000_000_001,
                class: "retrieval".into(),
            },
        ];
        let (latency, result) = simulate(&fx, &arrivals, 10_000, 1_000);
        assert_eq!(result.completed, 2);
        assert_eq!(latency.len(), 10);
        assert!(latency.p50() > 0);
        println!(
            "offered={} completed={} histogram_samples={} p50={}",
            result.offered,
            result.completed,
            latency.len(),
            latency.p50()
        );
    }

    #[test]
    fn burst_generator_misses_short_high_rate_phases() {
        // Correct stationary CTMC rate: equal dwell means => (1 + 1000)/2 = 500.5/s.
        // Phase transitions are fast relative to either arrival rate.
        let cfg = BurstConfig {
            classes: vec!["retrieval".into()],
            count: 1_000,
            seed: 42,
            low_lambda: 1.0,
            high_lambda: 1_000.0,
            mean_off_secs: 0.001,
            mean_on_secs: 0.001,
            ..BurstConfig::default()
        };
        let arrivals = generate_bursty(&cfg);
        let rate = cfg.count as f64 / arrivals.last().unwrap().send_time_secs;
        println!("observed_rate={rate:.6}/s expected_stationary_rate=500.5/s");
        assert!(
            rate < 10.0,
            "currently generated rate is dominated by old-phase gaps: {rate}"
        );
    }
}
