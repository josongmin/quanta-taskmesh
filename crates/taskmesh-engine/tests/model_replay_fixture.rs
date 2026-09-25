#![cfg(shuttle)]

use std::any::Any;
use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use shuttle::scheduler::RandomScheduler;
use shuttle::thread;
use shuttle::{Config, FailurePersistence, Runner};
use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet};

const FAILURE_MODEL_ID: &str = "shuttle.governor_replay_fixture.v1";
const FAILURE_SEED: u64 = 0x5a17_fa11;
const SENTINEL: &str = "taskmesh-governor-replay-sentinel";

fn schedule_sensitive_failure() {
    let class = TaskClass::new("replay");
    let governor = Arc::new(
        Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(1),
                BTreeMap::from([(
                    class.clone(),
                    ClassPolicy::new()
                        .max_inflight(1)
                        .max_queue_depth(1)
                        .cpu_units(1)
                        .overflow_policy(OverflowPolicy::QueueWithinDepth),
                )]),
            ),
            Arc::new(ManualClock::new(1_000)),
        )
        .expect("valid one-slot policy"),
    );
    let first_governor = Arc::clone(&governor);
    let first_class = class.clone();
    let first = thread::spawn(move || {
        // This yield makes the documented first-submitter-wins assumption
        // schedule-sensitive while the real Governor resolves both calls.
        thread::yield_now();
        first_governor.admit(&TaskSpec::io(first_class).operation("first"))
    });
    let second_governor = Arc::clone(&governor);
    let second =
        thread::spawn(move || second_governor.admit(&TaskSpec::io(class).operation("second")));
    let first_outcome = first.join().expect("first model thread joins");
    let second_outcome = second.join().expect("second model thread joins");
    assert!(
        matches!(first_outcome, AdmissionDecision::Admitted { .. })
            && matches!(second_outcome, AdmissionDecision::Queued { .. }),
        "{SENTINEL}: intentionally false first-submitter-wins assumption: \
         first={first_outcome:?} second={second_outcome:?}"
    );
}

fn panic_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic")
}

fn only_schedule(directory: &Path) -> PathBuf {
    let schedules: Vec<_> = fs::read_dir(directory)
        .expect("failure directory is readable")
        .map(|entry| entry.expect("directory entry is readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .collect();
    assert_eq!(schedules.len(), 1, "expected exactly one failure schedule");
    schedules.into_iter().next().expect("one schedule exists")
}

#[test]
fn failure_schedule_replays_the_same_failure() {
    let directory = PathBuf::from(
        std::env::var_os("TASKMESH_MODEL_FAILURE_DIR")
            .expect("TASKMESH_MODEL_FAILURE_DIR must name an isolated empty directory"),
    );
    fs::create_dir_all(&directory).expect("failure directory can be created");
    match std::env::var("TASKMESH_MODEL_REPLAY_MODE").as_deref() {
        Ok("generate") => {
            assert!(
                fs::read_dir(&directory)
                    .expect("failure directory is readable")
                    .next()
                    .is_none(),
                "generation requires an empty failure directory"
            );
            let mut config = Config::new();
            config.failure_persistence = FailurePersistence::File(Some(directory.clone()));
            let first = catch_unwind(AssertUnwindSafe(|| {
                Runner::new(RandomScheduler::new_from_seed(FAILURE_SEED, 100), config)
                    .run(schedule_sensitive_failure);
            }))
            .expect_err("fixture model must fail");
            assert!(panic_text(first.as_ref()).contains(SENTINEL));
            let schedule = only_schedule(&directory);
            // nosemgrep: taskmesh-test-eprintln-debug-leftover -- reason: modelcheck consumes this generation witness.
            println!(
                "taskmesh-replay-generation model_id={FAILURE_MODEL_ID} seed={FAILURE_SEED} \
                 result=failure_persisted artifact={}",
                schedule.display()
            );
        }
        Ok("replay") => {
            let schedule = only_schedule(&directory);
            let replay = catch_unwind(AssertUnwindSafe(|| {
                shuttle::replay_from_file(schedule_sensitive_failure, &schedule);
            }))
            .expect_err("saved schedule must reproduce the failure");
            assert!(panic_text(replay.as_ref()).contains(SENTINEL));
            // nosemgrep: taskmesh-test-eprintln-debug-leftover -- reason: modelcheck consumes this replay witness.
            println!(
                "taskmesh-replay-witness model_id={FAILURE_MODEL_ID} seed={FAILURE_SEED} \
                 result=same_failure artifact={}",
                schedule.display()
            );
        }
        Ok(other) => panic!("unknown TASKMESH_MODEL_REPLAY_MODE {other}"),
        Err(_) => panic!("TASKMESH_MODEL_REPLAY_MODE must be generate or replay"),
    }
}
