#![cfg(shuttle)]

use std::any::Any;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use shuttle::scheduler::RandomScheduler;
use shuttle::{Config, FailurePersistence, Runner};

const FAILURE_MODEL_ID: &str = "shuttle.replay_fixture.v1";
const FAILURE_SEED: u64 = 0x5a17_fa11;
const SENTINEL: &str = "taskmesh-v03-replay-sentinel";

fn deliberate_failure() {
    panic!("{SENTINEL}");
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
                Runner::new(RandomScheduler::new_from_seed(FAILURE_SEED, 1), config)
                    .run(deliberate_failure);
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
                shuttle::replay_from_file(deliberate_failure, &schedule);
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
