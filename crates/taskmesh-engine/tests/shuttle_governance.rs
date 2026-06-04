//! Randomized concurrency model-check (ADR 9000 / P6).
//!
//! loom (`loom_governance.rs`) explores *every* interleaving but only for tiny
//! state (2 threads). shuttle complements it with a randomized scheduler that
//! scales to larger state — more threads and more ops — sampling thousands of
//! schedules. Same modeled design as loom: a Relaxed atomic id taken OUTSIDE the
//! lock, then committed under one global mutex (mirrors `Governor::fresh_ids` +
//! `GovernedState::grant`/`unwind`).
//!
//! Run with: `RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --test shuttle_governance --release`.

#![cfg(shuttle)]

use std::collections::BTreeSet;

use shuttle::sync::atomic::{AtomicU64, Ordering};
use shuttle::sync::{Arc, Mutex};
use shuttle::thread;

#[derive(Default)]
struct State {
    inflight: i64,
    granted: BTreeSet<u64>,
}

fn admit(next_id: &AtomicU64, state: &Mutex<State>) -> u64 {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let mut s = state.lock().unwrap();
    assert!(s.granted.insert(id), "permit id {id} double-granted");
    s.inflight += 1;
    id
}

fn release(state: &Mutex<State>, id: u64) {
    let mut s = state.lock().unwrap();
    assert!(s.granted.remove(&id), "release of unknown permit {id}");
    s.inflight -= 1;
}

#[test]
fn randomized_concurrent_admit_release_is_safe() {
    shuttle::check_random(
        || {
            let next_id = Arc::new(AtomicU64::new(1));
            let state = Arc::new(Mutex::new(State::default()));

            // 4 threads, each admitting and releasing twice — larger state than
            // loom can exhaust, sampled randomly.
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let next_id = next_id.clone();
                    let state = state.clone();
                    thread::spawn(move || {
                        for _ in 0..2 {
                            let id = admit(&next_id, &state);
                            release(&state, id);
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }

            let s = state.lock().unwrap();
            assert_eq!(s.inflight, 0, "inflight must return to zero");
            assert!(s.granted.is_empty(), "no permit may leak");
        },
        10_000,
    );
}

#[test]
fn randomized_overlapping_holders_are_consistent() {
    shuttle::check_random(
        || {
            let next_id = Arc::new(AtomicU64::new(1));
            let state = Arc::new(Mutex::new(State::default()));

            // One long holder overlaps several admit/release churners.
            let holder = {
                let next_id = next_id.clone();
                let state = state.clone();
                thread::spawn(move || {
                    let id = admit(&next_id, &state);
                    thread::yield_now();
                    release(&state, id);
                })
            };
            let churners: Vec<_> = (0..3)
                .map(|_| {
                    let next_id = next_id.clone();
                    let state = state.clone();
                    thread::spawn(move || {
                        let id = admit(&next_id, &state);
                        release(&state, id);
                    })
                })
                .collect();

            holder.join().unwrap();
            for c in churners {
                c.join().unwrap();
            }

            let s = state.lock().unwrap();
            assert_eq!(s.inflight, 0);
            assert!(s.granted.is_empty());
        },
        10_000,
    );
}
